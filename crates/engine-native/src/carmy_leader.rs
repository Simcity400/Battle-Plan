
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_2648);
pub const ATTACK: usize = 0xf58;
pub const CAPTURED: usize = 0x1059;
pub const DEFENSE: usize = 0xf68;
pub const PLANNING: usize = 0xf78;
pub const LOGISTICS: usize = 0xf88;
pub const FORMATION_REFERENCE: usize = 0x1048;
pub const CAPTOR_COUNTRY: usize = 0x105c;
pub const DEPLOYED_HQ: usize = 0x1068;
pub const SIZE: usize = 0x10b8;

pub const MOVEMENT_TERRAIN_DATA: usize = 0x248;
pub const MOVEMENT_TERRAIN_STRIDE: usize = 0x28;
pub const MOVEMENT_TERRAIN_SPEED: usize = 0x20;
pub const MOVEMENT_MODIFIER: usize = 0x408;
pub const SEPARATE_HQ_MOVEMENT_MODIFIER: usize = 0x4c8;
pub const MODIFIER_ACCUMULATOR: usize = 0x10;
pub const MOVEMENT_MODIFIER_KEYS: [u32; 6] = [
    engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
    engine_executable::modifier_keys::ARMY_ARMOR_SPEED_FACTOR,
    engine_executable::modifier_keys::OUT_OF_SUPPLY_FACTOR,
    engine_executable::modifier_keys::COMBAT_WIDTH_FACTOR,
    engine_executable::modifier_keys::RIVER_CROSSING_FACTOR,
    engine_executable::modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
];
const PRIMARY_ROOT_CACHE: usize = 0x20;
const PRIMARY_ROOT_DIRTY: usize = 0x28;

#[derive(Clone, Copy)]
struct PrimaryPrefix {
    root: usize,
    address: usize,
    header: [u8; 16],
    consumed: u32,
}

struct PrimaryChunk {
    bytes: [u8; 512],
    len: usize,
    slot: usize,
}

pub struct PrimaryRootCache {
    metadata: crate::carmy::PresentationInputs,
    prefixes: Vec<Option<PrimaryPrefix>>,
    chunks: Vec<PrimaryChunk>,
    index: Vec<u32>,
    chunk_limit: usize,
}

impl PrimaryRootCache {
    pub fn try_new(roots: usize, chunks: usize) -> std::io::Result<Self> {
        if chunks > (u32::MAX as usize - 1) / 64 {
            return Err(engine_allocation::oom());
        }
        Ok(Self {
            metadata: crate::carmy::PresentationInputs::new(if chunks == 0 { 0 } else { roots })?,
            prefixes: engine_allocation::filled(roots, None)?,
            chunks: engine_allocation::vec(chunks)?,
            index: engine_allocation::filled(
                chunks.checked_mul(128).ok_or_else(engine_allocation::oom)?,
                0,
            )?,
            chunk_limit: chunks,
        })
    }

    #[cfg(test)]
    pub fn new(roots: usize, chunks: usize) -> Self {
        Self::try_new(roots, chunks).unwrap()
    }

    pub fn vacant() -> Self {
        Self {
            metadata: crate::carmy::PresentationInputs::vacant(),
            prefixes: Vec::new(),
            chunks: Vec::new(),
            index: Vec::new(),
            chunk_limit: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.metadata.allocated_bytes()
            + self.prefixes.capacity() * std::mem::size_of::<Option<PrimaryPrefix>>()
            + self.chunks.capacity() * std::mem::size_of::<PrimaryChunk>()
            + self.index.capacity() * std::mem::size_of::<u32>()
    }

    pub fn clear(&mut self) {
        self.metadata.clear();
        self.prefixes.fill(None);
        self.chunks.clear();
        self.index.fill(0);
    }

    pub fn grow(&mut self, roots: usize, chunks: usize) -> std::io::Result<()> {
        if roots > self.prefixes.len() || chunks > self.chunk_limit {
            *self = Self::try_new(roots.max(self.prefixes.len()), chunks.max(self.chunk_limit))?;
        } else {
            self.clear();
        }
        Ok(())
    }

    pub fn resolve(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        member: usize,
    ) -> Result<usize, &'static str> {
        movement_primary_root_internal(memory, base, member, Some(self))
    }

    fn hash(slot: usize, member: u64) -> usize {
        let mut value = member ^ (slot as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        (value ^ (value >> 27)) as usize
    }

    fn word(&self, code: u32) -> (usize, u64) {
        let ordinal = code as usize - 1;
        let chunk = &self.chunks[ordinal / 64];
        let start = (ordinal % 64) * 8;
        (
            chunk.slot,
            u64::from_le_bytes(chunk.bytes[start..start + 8].try_into().unwrap()),
        )
    }

    fn index_slot(&self, slot: usize, member: u64) -> usize {
        let mut index = Self::hash(slot, member) % self.index.len();
        loop {
            let code = self.index[index];
            if code == 0 || self.word(code) == (slot, member) {
                return index;
            }
            index += 1;
            if index == self.index.len() {
                index = 0;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn contains(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        root: usize,
        address: usize,
        header: [u8; 16],
        members: engine_native_memory::cpdx_array::Array,
        member: usize,
    ) -> Result<bool, &'static str> {
        if self.prefixes.is_empty() || self.chunk_limit == 0 {
            return primary_contains_from(memory, members, member, 0);
        }
        let slot = Self::hash(0, root as u64) % self.prefixes.len();
        let mut first = match self.prefixes[slot] {
            Some(prefix) if prefix.root != root => {
                return primary_contains_from(memory, members, member, 0);
            }
            Some(prefix) => {
                if prefix.address != address || prefix.header != header {
                    return Err("movement assigned members changed");
                }
                if self.index[self.index_slot(slot, member as u64)] != 0 {
                    return Ok(true);
                }
                prefix.consumed
            }
            None => {
                if self.chunks.len() == self.chunk_limit {
                    return primary_contains_from(memory, members, member, 0);
                }
                self.prefixes[slot] = Some(PrimaryPrefix {
                    root,
                    address,
                    header,
                    consumed: 0,
                });
                0
            }
        };
        while first < members.count {
            if self.chunks.len() == self.chunk_limit {
                return primary_contains_from(memory, members, member, first);
            }
            let count = (members.count - first).min(64) as usize;
            let address = members
                .element(first, 8)
                .ok_or("movement assigned members overflow")?;
            let mut chunk = PrimaryChunk {
                bytes: [0; 512],
                len: count * 8,
                slot,
            };
            if !engine_native_memory::read_slice(memory, address, &mut chunk.bytes[..chunk.len]) {
                return Err("movement assigned members unreadable");
            }
            let found = chunk.bytes[..chunk.len]
                .chunks_exact(8)
                .any(|word| u64::from_le_bytes(word.try_into().unwrap()) == member as u64);
            let ordinal = self.chunks.len();
            self.chunks.push(chunk);
            for word in 0..count {
                let code = (ordinal * 64 + word + 1) as u32;
                let (_, value) = self.word(code);
                let index = self.index_slot(slot, value);
                if self.index[index] == 0 {
                    self.index[index] = code;
                }
            }
            first += count as u32;
            self.prefixes[slot].as_mut().unwrap().consumed = first;
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn primary_contains_from(
    memory: &impl engine_native_memory::Memory,
    members: engine_native_memory::cpdx_array::Array,
    member: usize,
    mut first: u32,
) -> Result<bool, &'static str> {
    let mut bytes = [0; 512];
    while first < members.count {
        let count = (members.count - first).min(64) as usize;
        let address = members
            .element(first, 8)
            .ok_or("movement assigned members overflow")?;
        if !engine_native_memory::read_slice(memory, address, &mut bytes[..count * 8]) {
            return Err("movement assigned members unreadable");
        }
        if bytes[..count * 8]
            .chunks_exact(8)
            .any(|word| u64::from_le_bytes(word.try_into().unwrap()) == member as u64)
        {
            return Ok(true);
        }
        first += count as u32;
    }
    Ok(false)
}

const MEMO_ALLOCATION: &str = "movement leader memo allocation failed";

fn primary_shared<T: engine_native_memory::memory::Raw>(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    offset: usize,
    cache: &mut Option<&mut PrimaryRootCache>,
) -> Result<T, &'static str> {
    let address = movement_at(pointer, offset)?;
    match cache.as_deref_mut() {
        Some(cache) => cache.metadata.read(memory, address).map_err(|_| MEMO_ALLOCATION)?,
        None => engine_native_memory::read(memory, address),
    }
    .ok_or("movement leader field unreadable")
}

fn movement_at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("movement leader address overflow")
}

fn movement_field<T: engine_native_memory::memory::Raw>(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    engine_native_memory::read(memory, movement_at(pointer, offset)?).ok_or("movement leader field unreadable")
}

fn movement_global<T: engine_native_memory::memory::Raw>(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    let at = engine_sites::at(base, key).ok_or("movement leader address overflow")?;
    engine_native_memory::read(memory, at).ok_or("movement leader field unreadable")
}

fn movement_batched<T: engine_native_memory::memory::Raw, const N: usize>(
    memory: &impl engine_native_memory::Memory,
    batch: &engine_native_memory::memory::FieldBatch<N>,
    index: usize,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    movement_at(pointer, offset)?;
    batch
        .get(memory, index)
        .ok_or("movement leader field unreadable")
}

fn movement_fields<const N: usize>(
    pointer: usize,
    fields: [(usize, usize); N],
) -> [(usize, usize); N] {
    fields.map(|(offset, len)| (pointer.checked_add(offset).unwrap_or(0), len))
}

fn movement_group_leader(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    group: usize,
) -> Result<usize, &'static str> {
    use crate::corders_group::{HANDLE, LEADER, REFERENCE_TYPE};
    let batch = engine_native_memory::memory::FieldBatch::gather(
        memory,
        movement_fields(group, [(0, 8), (HANDLE, 4), (LEADER, 8)]),
    );
    let vtable = movement_batched::<u64, 3>(memory, &batch, 0, group, 0)?;
    if engine_sites::which(
        base,
        vtable as usize,
        &[
            crate::corders_group::VTABLE_RVA,
            engine_native_parts::carmy_group::VTABLE_RVA,
        ],
    )
    .is_none()
        || movement_batched::<u32, 3>(memory, &batch, 1, group, HANDLE)? != REFERENCE_TYPE
    {
        return Err("movement leader group identity mismatch");
    }
    Ok(movement_batched::<u64, 3>(memory, &batch, 2, group, LEADER)? as usize)
}

fn movement_leader_identity<const N: usize>(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    leader: usize,
    batch: &engine_native_memory::memory::FieldBatch<N>,
) -> Result<(), &'static str> {
    if movement_batched::<u64, N>(memory, batch, 0, leader, 0)?
        != engine_sites::at(base, VTABLE_RVA).ok_or("movement leader address overflow")? as u64
        || movement_batched::<u32, N>(memory, batch, 1, leader, crate::cunit_leader::TYPE)?
            != crate::cunit_leader::REFERENCE_TYPE
    {
        return Err("movement army leader identity mismatch");
    }
    Ok(())
}

fn movement_primary_root(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    member: usize,
) -> Result<usize, &'static str> {
    if let Some(result) = memory.movement_primary_root(base, member) {
        return result;
    }
    movement_primary_root_internal(memory, base, member, None)
}

fn movement_primary_root_internal(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    member: usize,
    mut cache: Option<&mut PrimaryRootCache>,
) -> Result<usize, &'static str> {
    use crate::{corder_instance as order, corders_group as group, corders_group_member};
    use engine_native_memory::cpdx_array::Array;
    let own = movement_field::<u64>(memory, member, corders_group_member::GROUP)? as usize;
    let dirty = movement_field::<u8>(memory, member, PRIMARY_ROOT_DIRTY)?;
    let cached = movement_field::<u64>(memory, member, PRIMARY_ROOT_CACHE)? as usize;
    let mut expected = 0;
    let authenticate =
        |pointer, cache: &mut Option<&mut PrimaryRootCache>| -> Result<(), &'static str> {
            let vtable = primary_shared::<u64>(memory, pointer, 0, cache)?;
            if engine_sites::which(
                base,
                vtable as usize,
                &[group::VTABLE_RVA, engine_native_parts::carmy_group::VTABLE_RVA],
            )
            .is_none()
                || primary_shared::<u32>(memory, pointer, group::HANDLE, cache)?
                    != group::REFERENCE_TYPE
            {
                return Err("movement leader group identity mismatch");
            }
            Ok(())
        };
    if own != 0 {
        authenticate(own, &mut cache)?;
        let parent = movement_field::<u64>(memory, own, group::PARENT)? as usize;
        for owner in [own, parent] {
            if owner == 0 {
                continue;
            }
            authenticate(owner, &mut cache)?;
            let at = movement_at(owner, group::PRIMARY_ORDER_DATA)?;
            let roots = match cache.as_deref_mut() {
                Some(cache) => cache
                    .metadata
                    .read(memory, at)
                    .map_err(|_| MEMO_ALLOCATION)?
                    .and_then(|header| Array::from_header(header, 8)),
                None => Array::read(memory, at, 8),
            }
            .ok_or("movement primary roots invalid")?;
            for index in 0..roots.count {
                let root = primary_shared::<u64>(
                    memory,
                    roots
                        .element(index, 8)
                        .ok_or("movement primary root overflow")?,
                    0,
                    &mut cache,
                )? as usize;
                if root == 0
                    || primary_shared::<u64>(memory, root, 0, &mut cache)?
                        != engine_sites::at(base, order::VTABLE_RVA)
                            .ok_or("movement leader address overflow")?
                            as u64
                    || primary_shared::<u64>(memory, root, order::OWNER, &mut cache)?
                        != owner as u64
                {
                    return Err("movement primary root identity mismatch");
                }
                let members_at = movement_at(root, order::ASSIGNED_MEMBER_DATA)?;
                let header = engine_native_memory::read::<[u8; 16]>(memory, members_at)
                    .ok_or("movement assigned members invalid")?;
                let members =
                    Array::from_header(header, 8).ok_or("movement assigned members invalid")?;
                let found = match cache.as_deref_mut() {
                    Some(cache) => {
                        cache.contains(memory, root, members_at, header, members, member)?
                    }
                    None => primary_contains_from(memory, members, member, 0)?,
                };
                if found {
                    expected = root;
                }
                if expected != 0 {
                    break;
                }
            }
            if expected != 0 {
                break;
            }
        }
    }
    if dirty == 0 && cached != expected {
        return Err("movement primary root clean cache is stale");
    }
    Ok(expected)
}

pub fn movement_accumulator(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    army: usize,
) -> Result<Option<usize>, &'static str> {
    let own = movement_field::<u64>(memory, army, crate::carmy::ORDERS_GROUP)? as usize;
    movement_accumulator_internal(memory, base, army, own, || {
        movement_field::<u32>(memory, army, crate::cunit::CLASS)
    })
}

pub fn movement_accumulator_seeded(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
) -> Result<Option<usize>, &'static str> {
    movement_at(seed.pointer, crate::carmy::ORDERS_GROUP)?;
    let own = seed
        .orders_group
        .ok_or("movement leader field unreadable")?;
    movement_accumulator_internal(memory, base, seed.pointer, own, || {
        movement_at(seed.pointer, crate::cunit::CLASS)?;
        seed.class.ok_or("movement leader field unreadable")
    })
}

fn movement_accumulator_internal(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    army: usize,
    own: usize,
    class: impl FnOnce() -> Result<u32, &'static str>,
) -> Result<Option<usize>, &'static str> {
    if own == 0 {
        return Ok(None);
    }
    let leader = movement_leader(memory, base, own)?;
    if leader == 0 {
        return Ok(None);
    }
    let mut alternate = false;
    if class()? == 0
        && movement_global::<u32>(
            memory,
            base,
            engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
        )? & (1 << 17)
            != 0
        && let Some(hq) = movement_hq_root(memory, base, own)?
    {
        alternate = hq != movement_own_root(memory, base, army)?;
    }
    movement_selected(leader, alternate)
}

pub fn separate_hq_order(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    unit: usize,
) -> Result<bool, &'static str> {
    if movement_field::<u32>(memory, unit, crate::cunit::CLASS)? != 0
        || movement_global::<u32>(
            memory,
            base,
            engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
        )? & (1 << 17)
            == 0
    {
        return Ok(false);
    }
    let own = movement_field::<u64>(memory, unit, crate::carmy::ORDERS_GROUP)? as usize;
    if own == 0 || movement_group_leader(memory, base, own)? == 0 {
        return Ok(false);
    }
    let Some(hq) = movement_hq_root(memory, base, own)? else {
        return Ok(false);
    };
    Ok(hq != movement_own_root(memory, base, unit)?)
}

fn movement_leader(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    group: usize,
) -> Result<usize, &'static str> {
    let leader = movement_group_leader(memory, base, group)?;
    if leader == 0 {
        return Ok(0);
    }
    let batch = engine_native_memory::memory::FieldBatch::gather(
        memory,
        [
            (leader, 8),
            (
                leader.checked_add(crate::cunit_leader::TYPE).unwrap_or(0),
                4,
            ),
        ],
    );
    movement_leader_identity(memory, base, leader, &batch)?;
    Ok(leader)
}

fn movement_hq_root(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    own: usize,
) -> Result<Option<usize>, &'static str> {
    let hq = movement_field::<u64>(memory, own, crate::corders_group::HQ_PROVIDER)? as usize;
    movement_hq_provider_root(memory, base, own, hq)
}

fn movement_hq_provider_root(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    own: usize,
    hq: usize,
) -> Result<Option<usize>, &'static str> {
    use crate::{carmy, corders_group_member, cunit};
    if hq == 0 {
        return Ok(None);
    }
    let host = hq
        .checked_sub(corders_group_member::HOST_OFFSET)
        .ok_or("movement HQ host underflow")?;
    if movement_field::<u64>(memory, host, 0)?
        != engine_sites::at(base, carmy::VTABLE_RVA).ok_or("movement leader address overflow")?
            as u64
        || movement_field::<u64>(memory, host, carmy::SECONDARY_BASE)?
            != engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA)
                .ok_or("movement leader address overflow")? as u64
        || movement_field::<u64>(memory, host, cunit::HANDLE)? == 0
        || movement_field::<u64>(memory, hq, corders_group_member::GROUP)? != own as u64
    {
        return Err("movement HQ host identity mismatch");
    }
    movement_primary_root(memory, base, hq).map(Some)
}

fn movement_own_root(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    army: usize,
) -> Result<usize, &'static str> {
    movement_primary_root(
        memory,
        base,
        movement_at(army, crate::corders_group_member::HOST_OFFSET)?,
    )
}

fn movement_selected(leader: usize, alternate: bool) -> Result<Option<usize>, &'static str> {
    let modifier = if alternate {
        SEPARATE_HQ_MOVEMENT_MODIFIER
    } else {
        MOVEMENT_MODIFIER
    };
    Ok(Some(movement_at(leader, modifier + MODIFIER_ACCUMULATOR)?))
}

pub(crate) struct PassTable<V: Copy> {
    slots: Vec<Option<PassEntry<V>>>,
    generation: u32,
    used: usize,
}

#[derive(Clone, Copy)]
struct PassEntry<V: Copy> {
    generation: u32,
    key: usize,
    value: V,
}

impl<V: Copy> PassTable<V> {
    pub(crate) fn try_with_capacity(keys: usize) -> std::io::Result<Self> {
        let slots = if keys == 0 {
            0
        } else {
            keys.checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(engine_allocation::oom)?
        };
        Ok(Self {
            slots: engine_allocation::filled(slots, None)?,
            generation: 1,
            used: 0,
        })
    }

    pub(crate) fn vacant() -> Self {
        Self {
            slots: Vec::new(),
            generation: 1,
            used: 0,
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.slots.len() / 2
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Option<PassEntry<V>>>()
    }

    pub(crate) fn clear(&mut self) {
        self.used = 0;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.slots.fill(None);
            self.generation = 1;
        }
    }

    pub(crate) fn get_or(&mut self, key: usize, compute: impl FnOnce() -> V) -> V {
        self.get_or_if(key, compute, |_| true)
    }

    pub(crate) fn get_or_if(
        &mut self,
        key: usize,
        compute: impl FnOnce() -> V,
        keep: impl FnOnce(V) -> bool,
    ) -> V {
        if self.slots.is_empty() {
            return compute();
        }
        let mask = self.slots.len() - 1;
        let hash = (key as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut at = ((hash ^ (hash >> 32)) as usize) & mask;
        loop {
            match self.slots[at] {
                Some(entry) if entry.generation == self.generation => {
                    if entry.key == key {
                        return entry.value;
                    }
                    at = (at + 1) & mask;
                }
                _ => break,
            }
        }
        let value = compute();
        if self.used < self.slots.len() / 2 && keep(value) {
            self.slots[at] = Some(PassEntry {
                generation: self.generation,
                key,
                value,
            });
            self.used += 1;
        }
        value
    }
}

pub struct LeaderMemo {
    leaders: PassTable<Result<usize, &'static str>>,
    hq: PassTable<Result<Option<usize>, &'static str>>,
    flags: Option<Result<u32, &'static str>>,
    roots: PrimaryRootCache,
}

impl LeaderMemo {
    pub fn try_with_capacity(groups: usize) -> std::io::Result<Self> {
        Ok(Self {
            leaders: PassTable::try_with_capacity(groups)?,
            hq: PassTable::try_with_capacity(groups)?,
            flags: None,
            roots: PrimaryRootCache::try_new(groups, groups.div_ceil(4))?,
        })
    }

    pub fn vacant() -> Self {
        Self {
            leaders: PassTable::vacant(),
            hq: PassTable::vacant(),
            flags: None,
            roots: PrimaryRootCache::vacant(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.leaders.capacity().min(self.hq.capacity())
    }

    pub fn allocated_bytes(&self) -> usize {
        self.leaders.allocated_bytes() + self.hq.allocated_bytes() + self.roots.allocated_bytes()
    }

    pub fn clear(&mut self) {
        self.leaders.clear();
        self.hq.clear();
        self.flags = None;
        self.roots.clear();
    }
}

#[derive(Clone, Copy, Default)]
pub struct MovementPreparation {
    leader: Option<Result<usize, &'static str>>,
    flags: Option<Result<u32, &'static str>>,
    hq: Option<Result<Option<usize>, &'static str>>,
    hq_provider: Option<usize>,
    own_root: Option<Result<usize, &'static str>>,
}
impl MovementPreparation {
    pub fn capture(
        memory: &impl engine_native_memory::Memory,
        base: usize,
        seed: &crate::carmy::DivisionSeed,
        memo: &mut LeaderMemo,
    ) -> Self {
        let mut prepared = Self::default();
        let _ = (|| {
            movement_at(seed.pointer, crate::carmy::ORDERS_GROUP)?;
            let own = seed
                .orders_group
                .ok_or("movement leader field unreadable")?;
            if own == 0 {
                return Ok(());
            }
            let leader = memo
                .leaders
                .get_or(own, || movement_leader(memory, base, own));
            prepared.leader = Some(leader);
            if leader? == 0 {
                return Ok(());
            }
            movement_at(seed.pointer, crate::cunit::CLASS)?;
            if seed.class.ok_or("movement leader field unreadable")? == 0 {
                let flags = *memo.flags.get_or_insert_with(|| {
                    movement_global::<u32>(
                        memory,
                        base,
                        engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
                    )
                });
                prepared.flags = Some(flags);
                if flags? & (1 << 17) != 0 {
                    let hq = memo.hq.get_or(own, || movement_hq_root(memory, base, own));
                    prepared.hq = Some(hq);
                    if hq?.is_some() {
                        let member =
                            movement_at(seed.pointer, crate::corders_group_member::HOST_OFFSET)?;
                        prepared.own_root =
                            Some(match memory.movement_primary_root(base, member) {
                                Some(root) => root,
                                None => memo.roots.resolve(memory, base, member),
                            });
                    }
                }
            }
            Ok::<_, &'static str>(())
        })();
        prepared
    }

    pub fn leader(&self) -> Option<usize> {
        self.leader
            .and_then(Result::ok)
            .filter(|&leader| leader != 0)
    }
}

pub fn movement_accumulator_memoized(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
    memo: &mut LeaderMemo,
) -> Result<Option<usize>, &'static str> {
    movement_block_memoized(memory, base, seed, memo).map(|block| block.map(|(root, _)| root))
}

pub fn movement_block_memoized(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
    memo: &mut LeaderMemo,
) -> Result<Option<(usize, bool)>, &'static str> {
    movement_block_prepared(memory, base, seed, memo, None)
}

pub fn movement_block_prepared(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
    memo: &mut LeaderMemo,
    prepared: Option<MovementPreparation>,
) -> Result<Option<(usize, bool)>, &'static str> {
    movement_at(seed.pointer, crate::carmy::ORDERS_GROUP)?;
    let own = seed
        .orders_group
        .ok_or("movement leader field unreadable")?;
    if own == 0 {
        return Ok(None);
    }
    let leader = memo.leaders.get_or(own, || {
        prepared
            .and_then(|p| p.leader)
            .unwrap_or_else(|| movement_leader(memory, base, own))
    })?;
    if leader == 0 {
        return Ok(None);
    }
    movement_at(seed.pointer, crate::cunit::CLASS)?;
    let class = seed.class.ok_or("movement leader field unreadable")?;
    let mut alternate = false;
    if class == 0 {
        let flags = *memo.flags.get_or_insert_with(|| {
            prepared.and_then(|p| p.flags).unwrap_or_else(|| {
                movement_global::<u32>(
                    memory,
                    base,
                    engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
                )
            })
        });
        if flags? & (1 << 17) != 0
            && let Some(hq) = memo.hq.get_or(own, || {
                prepared.and_then(|p| p.hq).unwrap_or_else(|| {
                    match prepared.and_then(|p| p.hq_provider) {
                        Some(provider) => movement_hq_provider_root(memory, base, own, provider),
                        None => movement_hq_root(memory, base, own),
                    }
                })
            })?
        {
            let member = movement_at(seed.pointer, crate::corders_group_member::HOST_OFFSET)?;
            let own = prepared.and_then(|p| p.own_root).unwrap_or_else(|| {
                match memory.movement_primary_root(base, member) {
                    Some(root) => root,
                    None => memo.roots.resolve(memory, base, member),
                }
            })?;
            alternate = hq != own;
        }
    }
    movement_selected(leader, alternate).map(|root| root.map(|root| (root, alternate)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    pub leader: Option<Result<usize, &'static str>>,
    pub handle: Option<u64>,
    pub group_identity: bool,
    pub block: Result<Option<(usize, bool)>, &'static str>,
    pub training: Option<Result<u8, &'static str>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SelectionCounts {
    pub groups: u64,
    pub carried_groups: u64,
    pub leaders: u64,
    pub carried: u64,
    pub calls: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupFields {
    pub leader: usize,
    pub training: u8,
    pub hq_provider: usize,
}

pub trait SelectionCarry {
    fn leader(&mut self, leader: usize) -> Option<u64>;
    fn group(&mut self, group: usize) -> Option<GroupFields>;
    fn group_read(&mut self, group: usize, fields: GroupFields);
}

pub struct Fresh;
impl SelectionCarry for Fresh {
    fn leader(&mut self, _: usize) -> Option<u64> {
        None
    }
    fn group(&mut self, _: usize) -> Option<GroupFields> {
        None
    }
    fn group_read(&mut self, _: usize, _: GroupFields) {}
}

const SELECTION_BATCH: usize = 64;
const SELECTION_GROUP_SPANS: [(usize, usize); 5] = [
    (0, 8),
    (crate::corders_group::HANDLE, 8),
    (crate::corders_group::HQ_PROVIDER, 8),
    (crate::corders_group::LEADER, 8),
    (crate::corders_group::TRAINING, 1),
];
const SELECTION_GROUP_FIELDS: [engine_native_memory::memory::RecordField; 5] =
    engine_native_memory::memory::packed_fields(SELECTION_GROUP_SPANS);
const SELECTION_GROUP_STRIDE: usize = engine_native_memory::memory::packed_stride(SELECTION_GROUP_SPANS);
const SELECTION_GROUP_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&SELECTION_GROUP_FIELDS, SELECTION_GROUP_STRIDE)
        .expect("valid selection group layout");
const SELECTION_LEADER_FIELDS: [engine_native_memory::memory::RecordField; 2] =
    engine_native_memory::memory::packed_fields([(0, 8), (crate::cunit_leader::TYPE, 8)]);
const SELECTION_LEADER_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&SELECTION_LEADER_FIELDS, 16)
        .expect("valid selection leader layout");

#[derive(Clone, Copy)]
struct GroupSelection {
    group: usize,
    leader: Result<usize, &'static str>,
    handle: Option<u64>,
    identity: bool,
    training: Result<u8, &'static str>,
    hq_provider: Option<usize>,
}

fn leader_handle(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    leader: usize,
) -> Result<Option<u64>, &'static str> {
    let batch = engine_native_memory::memory::FieldBatch::gather(
        memory,
        movement_fields(leader, [(0, 8), (crate::cunit_leader::TYPE, 4)]),
    );
    movement_leader_identity(memory, base, leader, &batch)?;
    Ok(movement_field::<u64>(memory, leader, crate::cunit_leader::TYPE).ok())
}

pub fn capture_selections(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    seeds: &mut [crate::carmy::DivisionSeed],
    carry: &mut dyn SelectionCarry,
) -> Result<SelectionCounts, &'static str> {
    const ALLOCATION: &str = "division selection allocation failed";
    let mut counts = SelectionCounts::default();
    let mut groups: Vec<GroupSelection> = Vec::new();
    engine_allocation::advisory(|| engine_allocation::reserve(&mut groups, seeds.len()))
        .map_err(|_| ALLOCATION)?;
    groups.extend(
        seeds
            .iter()
            .filter_map(|seed| seed.orders_group.filter(|&group| group != 0))
            .map(|group| GroupSelection {
                group,
                leader: Ok(0),
                handle: None,
                identity: false,
                training: Ok(0),
                hq_provider: None,
            }),
    );
    groups.sort_unstable_by_key(|entry| entry.group);
    groups.dedup_by_key(|entry| entry.group);
    let group_vtables = [
        crate::corders_group::VTABLE_RVA,
        engine_native_parts::carmy_group::VTABLE_RVA,
    ]
    .map(|rva| engine_sites::at(base, rva).map(|vtable| vtable as u64));
    let mut pointers = [0_usize; SELECTION_BATCH];
    let mut rows = [0_usize; SELECTION_BATCH];
    let mut complete = [false; SELECTION_BATCH];
    let mut group_bytes = [0_u8; SELECTION_BATCH * SELECTION_GROUP_STRIDE];
    for chunk in groups.chunks_mut(SELECTION_BATCH) {
        let mut count = 0;
        for (row, entry) in chunk.iter_mut().enumerate() {
            counts.groups += 1;
            match carry.group(entry.group) {
                Some(fields) => {
                    counts.carried_groups += 1;
                    entry.identity = true;
                    entry.leader = Ok(fields.leader);
                    entry.training = Ok(fields.training);
                    entry.hq_provider = Some(fields.hq_provider);
                }
                None => {
                    pointers[count] = entry.group;
                    rows[count] = row;
                    count += 1;
                }
            }
        }
        if count == 0 {
            continue;
        }
        engine_native_memory::memory::gather_records(
            memory,
            &pointers[..count],
            &SELECTION_GROUP_LAYOUT,
            &mut group_bytes[..count * SELECTION_GROUP_STRIDE],
            &mut complete[..count],
        );
        counts.calls += 1;
        for read in 0..count {
            let entry = &mut chunk[rows[read]];
            if !complete[read] {
                counts.calls += 2;
                entry.leader = movement_group_leader(memory, base, entry.group);
                entry.training = entry
                    .group
                    .checked_add(crate::corders_group::TRAINING)
                    .and_then(|at| engine_native_memory::read::<u8>(memory, at))
                    .ok_or("division presentation field unreadable");
                continue;
            }
            let record = &group_bytes[read * SELECTION_GROUP_STRIDE..][..SELECTION_GROUP_STRIDE];
            let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
            let kind = u32::from_le_bytes(record[8..12].try_into().unwrap());
            entry.identity = group_vtables.contains(&Some(word(0)))
                && kind == crate::corders_group::REFERENCE_TYPE;
            entry.training = Ok(record[32]);
            if !entry.identity {
                entry.leader = Err("movement leader group identity mismatch");
                continue;
            }
            let fields = GroupFields {
                leader: word(24) as usize,
                training: record[32],
                hq_provider: word(16) as usize,
            };
            entry.leader = Ok(fields.leader);
            entry.hq_provider = Some(fields.hq_provider);
            carry.group_read(entry.group, fields);
        }
    }
    let mut leaders: Vec<(usize, Result<Option<u64>, &'static str>)> = Vec::new();
    engine_allocation::advisory(|| engine_allocation::reserve(&mut leaders, groups.len()))
        .map_err(|_| ALLOCATION)?;
    leaders.extend(
        groups
            .iter()
            .filter_map(|entry| entry.leader.ok().filter(|&leader| leader != 0))
            .map(|leader| (leader, Err("movement leader field unreadable"))),
    );
    leaders.sort_unstable_by_key(|entry| entry.0);
    leaders.dedup_by_key(|entry| entry.0);
    let identity =
        engine_sites::at(base, VTABLE_RVA).ok_or("movement leader address overflow")? as u64;
    let mut pending = [0_usize; SELECTION_BATCH];
    let mut leader_bytes = [0_u8; SELECTION_BATCH * 16];
    let mut next = 0;
    while next < leaders.len() {
        let mut count = 0;
        while next < leaders.len() && count < SELECTION_BATCH {
            let leader = leaders[next].0;
            counts.leaders += 1;
            match carry.leader(leader) {
                Some(handle) => {
                    counts.carried += 1;
                    leaders[next].1 = Ok(Some(handle));
                }
                None => {
                    pending[count] = leader;
                    rows[count] = next;
                    count += 1;
                }
            }
            next += 1;
        }
        if count == 0 {
            continue;
        }
        engine_native_memory::memory::gather_records(
            memory,
            &pending[..count],
            &SELECTION_LEADER_LAYOUT,
            &mut leader_bytes[..count * 16],
            &mut complete[..count],
        );
        counts.calls += 1;
        for row in 0..count {
            let leader = pending[row];
            leaders[rows[row]].1 = if complete[row] {
                let record = &leader_bytes[row * 16..row * 16 + 16];
                let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
                let handle = u64::from_le_bytes(record[8..].try_into().unwrap());
                if vtable != identity || handle as u32 != crate::cunit_leader::REFERENCE_TYPE {
                    Err("movement army leader identity mismatch")
                } else {
                    Ok(Some(handle))
                }
            } else {
                let result = leader_handle(memory, base, leader);
                counts.calls += 1 + u64::from(result.is_ok());
                result
            };
        }
    }
    for entry in &mut groups {
        if let Ok(leader) = entry.leader
            && leader != 0
        {
            let at = leaders
                .binary_search_by_key(&leader, |entry| entry.0)
                .expect("every led group's leader was gathered");
            match leaders[at].1 {
                Ok(handle) => entry.handle = handle,
                Err(reason) => entry.leader = Err(reason),
            }
        }
    }
    let mut memo = engine_allocation::advisory(|| LeaderMemo::try_with_capacity(groups.len()))
        .map_err(|_| ALLOCATION)?;
    engine_allocation::advisory(|| {
        for seed in seeds.iter_mut() {
            let group = seed
                .orders_group
                .filter(|&group| group != 0)
                .and_then(|group| {
                    groups
                        .binary_search_by_key(&group, |entry| entry.group)
                        .ok()
                })
                .map(|at| groups[at]);
            let prepared = MovementPreparation {
                leader: group.map(|group| group.leader),
                hq_provider: group.and_then(|group| group.hq_provider),
                ..MovementPreparation::default()
            };
            seed.selected = Some(Selected {
                leader: group.map(|group| group.leader),
                handle: group.and_then(|group| group.handle),
                group_identity: group.is_some_and(|group| group.identity),
                block: movement_block_prepared(memory, base, seed, &mut memo, Some(prepared)),
                training: group.map(|group| group.training),
            });
        }
    });
    if memo.roots.metadata.refused() {
        for seed in seeds.iter_mut() {
            seed.selected = None;
        }
        return Err(ALLOCATION);
    }
    Ok(counts)
}

pub const CANNOT_RETREAT_WHILE_DEFENDING: u32 =
    engine_executable::modifier_keys::CANNOT_RETREAT_WHILE_DEFENDING;
pub const CANNOT_RETREAT_WHILE_ATTACKING: u32 =
    engine_executable::modifier_keys::CANNOT_RETREAT_WHILE_ATTACKING;
pub const RETREAT_BANNED_DEFENDING: u8 = 4;
pub const RETREAT_BANNED_ATTACKING: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetreatBans {
    pub leader: usize,
    pub ordinary: u8,
    pub alternate: u8,
    pub movement: [MovementEntries; 2],
}

pub type MovementEntries = [Option<i64>; MOVEMENT_MODIFIER_KEYS.len()];

impl RetreatBans {
    pub fn select(self, accumulator: usize) -> Result<u8, &'static str> {
        if accumulator == movement_at(self.leader, MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR)? {
            Ok(self.ordinary)
        } else if accumulator
            == movement_at(
                self.leader,
                SEPARATE_HQ_MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
            )?
        {
            Ok(self.alternate)
        } else {
            Err("retreat ban modifier block differs from the group's leader")
        }
    }
}

pub fn retreat_bans(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    group: usize,
) -> Result<Option<RetreatBans>, &'static str> {
    if group == 0 {
        return Ok(None);
    }
    let leader = movement_group_leader(memory, base, group)?;
    if leader == 0 {
        return Ok(None);
    }
    let batch = engine_native_memory::memory::FieldBatch::gather(
        memory,
        movement_fields(leader, [(0, 8), (crate::cunit_leader::TYPE, 4)]),
    );
    movement_leader_identity(memory, base, leader, &batch)?;
    let bans = |modifier: usize| -> Result<u8, &'static str> {
        let accumulator = movement_at(leader, modifier + MODIFIER_ACCUMULATOR)?;
        let mut bits = 0;
        for (key, bit) in [
            (CANNOT_RETREAT_WHILE_DEFENDING, RETREAT_BANNED_DEFENDING),
            (CANNOT_RETREAT_WHILE_ATTACKING, RETREAT_BANNED_ATTACKING),
        ] {
            if engine_native_parts::modifier_accumulator::lookup_effective_at(memory, base, accumulator, key)?
                .effective
                > 0
            {
                bits |= bit;
            }
        }
        Ok(bits)
    };
    let movement = |modifier: usize| -> Result<MovementEntries, &'static str> {
        let accumulator = movement_at(leader, modifier + MODIFIER_ACCUMULATOR)?;
        movement_entries(memory, accumulator)
    };
    Ok(Some(RetreatBans {
        leader,
        ordinary: bans(MOVEMENT_MODIFIER)?,
        alternate: bans(SEPARATE_HQ_MOVEMENT_MODIFIER)?,
        movement: [
            movement(MOVEMENT_MODIFIER)?,
            movement(SEPARATE_HQ_MOVEMENT_MODIFIER)?,
        ],
    }))
}

fn movement_entries(
    memory: &impl engine_native_memory::Memory,
    accumulator: usize,
) -> Result<MovementEntries, &'static str> {
    let mut values = [engine_native_memory::modifier_cache::Value::default(); MOVEMENT_MODIFIER_KEYS.len()];
    engine_native_parts::modifier_accumulator::lookup_many_at(
        memory,
        accumulator,
        &MOVEMENT_MODIFIER_KEYS,
        &mut values,
    )?;
    Ok(values.map(|value| value.present.then_some(value.raw)))
}

const BANS_BATCH: usize = 64;
const BANS_SPANS: usize = 16;
const BANS_GROUP_FIELDS: [engine_native_memory::memory::RecordField; 3] = [
    engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: crate::corders_group::HANDLE,
        destination_offset: 8,
        len: 4,
    },
    engine_native_memory::memory::RecordField {
        source_offset: crate::corders_group::LEADER,
        destination_offset: 12,
        len: 8,
    },
];
const BANS_GROUP_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&BANS_GROUP_FIELDS, 20).expect("valid group layout");
const BANS_LEADER_FIELDS: [engine_native_memory::memory::RecordField; 4] = [
    engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: crate::cunit_leader::TYPE,
        destination_offset: 8,
        len: 4,
    },
    engine_native_memory::memory::RecordField {
        source_offset: MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
        destination_offset: 12,
        len: 16,
    },
    engine_native_memory::memory::RecordField {
        source_offset: SEPARATE_HQ_MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
        destination_offset: 28,
        len: 16,
    },
];
const BANS_LEADER_BYTES: usize = 44;
const BANS_LEADER_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&BANS_LEADER_FIELDS, BANS_LEADER_BYTES)
        .expect("valid leader layout");

#[derive(Clone, Copy)]
struct BansLeader {
    group: usize,
    leader: usize,
    blocks: [engine_native_memory::cpdx_array::Array; 2],
    bits: [u8; 2],
    movement: [MovementEntries; 2],
    failure: Option<&'static str>,
}

fn bans_from_entries(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    entries: &[u8],
    memo: &engine_native_parts::modifier_definition::DefinitionMemo,
) -> Result<(u8, MovementEntries), &'static str> {
    let mut previous = 0;
    let mut bits = 0;
    let mut movement = [None; MOVEMENT_MODIFIER_KEYS.len()];
    for entry in entries.chunks_exact(engine_native_parts::modifier_entry::STRIDE) {
        let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
        if key <= previous {
            return Err("modifier accumulator keys not positive and increasing");
        }
        previous = key;
        let raw = i64::from_le_bytes(entry[8..].try_into().unwrap());
        if let Some(at) = MOVEMENT_MODIFIER_KEYS
            .iter()
            .position(|&wanted| wanted == key)
        {
            movement[at] = Some(raw);
        }
        let bit = match key {
            CANNOT_RETREAT_WHILE_DEFENDING => RETREAT_BANNED_DEFENDING,
            CANNOT_RETREAT_WHILE_ATTACKING => RETREAT_BANNED_ATTACKING,
            _ => continue,
        };
        if engine_native_parts::modifier_definition::effective_memo(memory, base, key, raw, memo)? > 0 {
            bits |= bit;
        }
    }
    Ok((bits, movement))
}

pub fn retreat_bans_many(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    groups: &[usize],
    known: &[Option<Result<usize, &'static str>>],
    output: &mut [Result<Option<RetreatBans>, &'static str>],
    memo: &engine_native_parts::modifier_definition::DefinitionMemo,
    held: &mut dyn FnMut(usize) -> Option<RetreatBans>,
) {
    use engine_native_memory::{cpdx_array::Array, memory::RecordSources};
    use engine_native_parts::modifier_entry::STRIDE;
    let expected = |rva: Rva| engine_sites::at(base, rva).map(|value| value as u64);
    let group_vtables = [
        expected(crate::corders_group::VTABLE_RVA),
        expected(engine_native_parts::carmy_group::VTABLE_RVA),
    ];
    let leader_vtable = expected(VTABLE_RVA);
    let mut group_bytes = [0_u8; BANS_BATCH * 20];
    let mut leader_bytes = [0_u8; BANS_BATCH * BANS_LEADER_BYTES];
    let mut entry_bytes = [0_u8; BANS_SPANS * BANS_BATCH * STRIDE];
    let mut start = 0;
    while start < groups.len() {
        let chunk = &groups[start..(start + BANS_BATCH).min(groups.len())];
        let mut chosen = [Ok(0_usize); BANS_BATCH];
        let mut unknown = [0_usize; BANS_BATCH];
        let mut unknown_at = [0_usize; BANS_BATCH];
        let mut unknowns = 0;
        for (index, &group) in chunk.iter().enumerate() {
            match known.get(start + index).copied().flatten() {
                Some(leader) => chosen[index] = leader,
                None => {
                    unknown[unknowns] = group;
                    unknown_at[unknowns] = index;
                    unknowns += 1;
                }
            }
        }
        let mut first = 0;
        while first < unknowns {
            let count = unknowns - first;
            let completed = memory.copy_records(
                RecordSources::Pointers(&unknown[first..unknowns]),
                &BANS_GROUP_LAYOUT,
                &mut group_bytes[..count * 20],
            );
            let read = (completed / BANS_GROUP_FIELDS.len()).min(count);
            for (offset, record) in group_bytes[..read * 20].chunks_exact(20).enumerate() {
                let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
                chosen[unknown_at[first + offset]] = if !group_vtables.contains(&Some(vtable))
                    || u32::from_le_bytes(record[8..12].try_into().unwrap())
                        != crate::corders_group::REFERENCE_TYPE
                {
                    Err("movement leader group identity mismatch")
                } else {
                    Ok(u64::from_le_bytes(record[12..20].try_into().unwrap()) as usize)
                };
            }
            if read < count {
                chosen[unknown_at[first + read]] = Err("movement leader field unreadable");
                first += read + 1;
            } else {
                first = unknowns;
            }
        }
        let mut leaders = [None::<BansLeader>; BANS_BATCH];
        let mut pending = 0;
        for (index, &leader) in chosen[..chunk.len()].iter().enumerate() {
            let slot = start + index;
            let leader = match leader {
                Err(reason) => {
                    output[slot] = Err(reason);
                    continue;
                }
                Ok(0) => {
                    output[slot] = Ok(None);
                    continue;
                }
                Ok(leader) => leader,
            };
            if let Some(bans) = held(leader) {
                output[slot] = Ok(Some(bans));
                continue;
            }
            leaders[pending] = Some(BansLeader {
                group: slot,
                leader,
                blocks: [Array { data: 0, count: 0 }; 2],
                bits: [0; 2],
                movement: [[None; MOVEMENT_MODIFIER_KEYS.len()]; 2],
                failure: None,
            });
            pending += 1;
        }
        start += chunk.len();
        let mut first = 0;
        while first < pending {
            let count = pending - first;
            let mut batch = [0_usize; BANS_BATCH];
            for (pointer, leader) in batch.iter_mut().zip(&leaders[first..pending]) {
                *pointer = leader.map_or(0, |leader| leader.leader);
            }
            let completed = memory.copy_records(
                RecordSources::Pointers(&batch[..count]),
                &BANS_LEADER_LAYOUT,
                &mut leader_bytes[..count * BANS_LEADER_BYTES],
            );
            let read = (completed / BANS_LEADER_FIELDS.len()).min(count);
            for (index, record) in leader_bytes[..read * BANS_LEADER_BYTES]
                .chunks_exact(BANS_LEADER_BYTES)
                .enumerate()
            {
                let leader = leaders[first + index].as_mut().unwrap();
                if Some(u64::from_le_bytes(record[..8].try_into().unwrap())) != leader_vtable
                    || u32::from_le_bytes(record[8..12].try_into().unwrap())
                        != crate::cunit_leader::REFERENCE_TYPE
                {
                    leader.failure = Some("movement army leader identity mismatch");
                    continue;
                }
                for (block, at) in [12, 28].into_iter().enumerate() {
                    match Array::from_header(record[at..at + 16].try_into().unwrap(), STRIDE) {
                        Some(array) => leader.blocks[block] = array,
                        None => {
                            leader.failure =
                                Some("modifier accumulator header unreadable or invalid");
                            break;
                        }
                    }
                }
            }
            if read < count {
                let leader = leaders[first + read].as_mut().unwrap();
                leader.failure = Some(if completed % BANS_LEADER_FIELDS.len() < 2 {
                    "movement leader field unreadable"
                } else {
                    "modifier accumulator header unreadable or invalid"
                });
                first += read + 1;
            } else {
                first += count;
            }
        }
        let mut spans = [(0_usize, 0_usize, 0_usize); BANS_SPANS];
        let mut queued = 0;
        let flush = |spans: &mut [(usize, usize, usize); BANS_SPANS],
                     queued: &mut usize,
                     leaders: &mut [Option<BansLeader>; BANS_BATCH],
                     entry_bytes: &mut [u8; BANS_SPANS * BANS_BATCH * STRIDE]| {
            let mut from = 0;
            while from < *queued {
                let count = *queued - from;
                let addresses: [usize; BANS_SPANS] = std::array::from_fn(|index| {
                    if index < count {
                        spans[from + index].0
                    } else {
                        0
                    }
                });
                let mut chunks = entry_bytes.chunks_exact_mut(BANS_BATCH * STRIDE);
                let mut destinations: [&mut [u8]; BANS_SPANS] = std::array::from_fn(|index| {
                    let chunk = chunks.next().unwrap();
                    let len = if index < count {
                        spans[from + index].1
                    } else {
                        0
                    };
                    &mut chunk[..len]
                });
                let copied = memory
                    .copy_scattered(addresses, &mut destinations)
                    .min(count);
                for (index, bytes) in entry_bytes
                    .chunks_exact(BANS_BATCH * STRIDE)
                    .take(copied)
                    .enumerate()
                {
                    let (_, len, key) = spans[from + index];
                    let leader = leaders[key / 2].as_mut().unwrap();
                    if leader.failure.is_some() {
                        continue;
                    }
                    match bans_from_entries(memory, base, &bytes[..len], memo) {
                        Ok((bits, movement)) => {
                            leader.bits[key % 2] = bits;
                            leader.movement[key % 2] = movement;
                        }
                        Err(reason) => leader.failure = Some(reason),
                    }
                }
                if copied < count {
                    let (_, _, key) = spans[from + copied];
                    let leader = leaders[key / 2].as_mut().unwrap();
                    leader
                        .failure
                        .get_or_insert("modifier accumulator entry unreadable");
                    from += copied + 1;
                } else {
                    from += count;
                }
            }
            *queued = 0;
        };
        for index in 0..pending {
            let leader = leaders[index].unwrap();
            if leader.failure.is_some() {
                continue;
            }
            for (block, array) in leader.blocks.into_iter().enumerate() {
                if array.count == 0 {
                    continue;
                }
                if array.count as usize > BANS_BATCH {
                    let mut values = [engine_native_memory::modifier_cache::Value::default(); 2];
                    let key = [
                        CANNOT_RETREAT_WHILE_DEFENDING,
                        CANNOT_RETREAT_WHILE_ATTACKING,
                    ];
                    let offset = [MOVEMENT_MODIFIER, SEPARATE_HQ_MOVEMENT_MODIFIER][block];
                    let result = movement_at(leader.leader, offset + MODIFIER_ACCUMULATOR)
                        .and_then(|at| {
                            engine_native_parts::modifier_accumulator::lookup_many_at(
                                memory,
                                at,
                                &key,
                                &mut values,
                            )
                        })
                        .and_then(|()| {
                            let mut bits = 0;
                            for ((value, key), bit) in values
                                .into_iter()
                                .zip(key)
                                .zip([RETREAT_BANNED_DEFENDING, RETREAT_BANNED_ATTACKING])
                            {
                                if value.present
                                    && engine_native_parts::modifier_definition::effective_memo(
                                        memory, base, key, value.raw, memo,
                                    )? > 0
                                {
                                    bits |= bit;
                                }
                            }
                            Ok(bits)
                        });
                    let movement = movement_at(leader.leader, offset + MODIFIER_ACCUMULATOR)
                        .and_then(|at| movement_entries(memory, at));
                    let leader = leaders[index].as_mut().unwrap();
                    match result.and_then(|bits| Ok((bits, movement?))) {
                        Ok((bits, movement)) => {
                            leader.bits[block] = bits;
                            leader.movement[block] = movement;
                        }
                        Err(reason) => {
                            leader.failure.get_or_insert(reason);
                        }
                    }
                    continue;
                }
                if queued == BANS_SPANS {
                    flush(&mut spans, &mut queued, &mut leaders, &mut entry_bytes);
                }
                spans[queued] = (array.data, array.count as usize * STRIDE, index * 2 + block);
                queued += 1;
            }
        }
        flush(&mut spans, &mut queued, &mut leaders, &mut entry_bytes);
        for leader in leaders[..pending].iter().flatten() {
            output[leader.group] = match leader.failure {
                Some(reason) => Err(reason),
                None => Ok(Some(RetreatBans {
                    leader: leader.leader,
                    ordinary: leader.bits[0],
                    alternate: leader.bits[1],
                    movement: leader.movement,
                })),
            };
        }
    }
}

pub fn movement_terrain(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    leader: usize,
    terrain_index: i32,
) -> Result<i64, &'static str> {
    let batch = engine_native_memory::memory::FieldBatch::gather(
        memory,
        movement_fields(
            leader,
            [
                (0, 8),
                (crate::cunit_leader::TYPE, 4),
                (MOVEMENT_TERRAIN_DATA, 16),
            ],
        ),
    );
    movement_leader_identity(memory, base, leader, &batch)?;
    movement_at(leader, MOVEMENT_TERRAIN_DATA)?;
    let array = batch
        .get::<[u8; 16]>(memory, 2)
        .and_then(|header| engine_native_memory::cpdx_array::Array::from_header(header, MOVEMENT_TERRAIN_STRIDE))
        .ok_or("movement leader terrain header invalid")?;
    if array.data == 0 {
        return Err("movement leader terrain data null");
    }
    let index = if terrain_index >= 0 && (terrain_index as u32) < array.count {
        terrain_index as usize
    } else {
        0
    };
    let pointer = movement_at(
        array.data,
        index
            .checked_mul(MOVEMENT_TERRAIN_STRIDE)
            .ok_or("movement leader terrain index overflow")?,
    )?;
    let speed = movement_field(memory, pointer, MOVEMENT_TERRAIN_SPEED)?;
    Ok(speed)
}

#[cfg(test)]
mod movement_tests;

pub const ATTACK_NIGHT: usize = 0x270;
pub const ATTACK_FORT: usize = 0xfb8;
pub const ATTACK_RIVER: usize = 0xfe0;
pub const ATTACK_SNOW: usize = 0x1030;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttackStats {
    pub leader: u64,
    pub terrain: Vec<i64>,
    pub fort: i64,
    pub river: i64,
    pub snow: i64,
    pub night: i64,
    pub movement_terrain: Vec<i64>,
}

pub fn attack_stats(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    leader: usize,
) -> Result<AttackStats, &'static str> {
    let handle = crate::cunit_leader::handle(memory, base, leader)?;
    let word = |offset: usize| -> Result<i64, &'static str> {
        leader
            .checked_add(offset)
            .and_then(|address| engine_native_memory::read::<i64>(memory, address))
            .ok_or("commander attack operand unreadable")
    };
    let mut scalars = [0_u8; ATTACK_SNOW + 8 - ATTACK_FORT];
    if !engine_native_memory::read_slice(
        memory,
        leader
            .checked_add(ATTACK_FORT)
            .ok_or("commander address overflow")?,
        &mut scalars,
    ) {
        return Err("commander attack operands unreadable");
    }
    let scalar = |offset: usize| {
        let at = offset - ATTACK_FORT;
        i64::from_le_bytes(scalars[at..at + 8].try_into().unwrap())
    };
    let header = engine_native_memory::read::<[u8; 16]>(
        memory,
        leader
            .checked_add(MOVEMENT_TERRAIN_DATA)
            .ok_or("commander address overflow")?,
    )
    .ok_or("commander terrain adjuster header unreadable")?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let count = i32::from_le_bytes(header[12..16].try_into().unwrap());
    let records = if data == 0 {
        0
    } else {
        usize::try_from(count).unwrap_or(0).max(1)
    };
    let mut terrain = Vec::new();
    engine_allocation::reserve_exact(&mut terrain, records)
        .map_err(|_| "commander terrain allocation failed")?;
    let mut movement_terrain = Vec::new();
    engine_allocation::reserve_exact(&mut movement_terrain, records)
        .map_err(|_| "commander terrain allocation failed")?;
    const BATCH: usize = 32;
    let mut entries = [0_u8; BATCH * MOVEMENT_TERRAIN_STRIDE];
    let mut first = 0;
    while first < records {
        let len = (records - first).min(BATCH);
        let address = first
            .checked_mul(MOVEMENT_TERRAIN_STRIDE)
            .and_then(|offset| data.checked_add(offset))
            .ok_or("commander terrain adjuster address overflow")?;
        if !engine_native_memory::read_slice(
            memory,
            address,
            &mut entries[..len * MOVEMENT_TERRAIN_STRIDE],
        ) {
            return Err("commander terrain adjusters unreadable");
        }
        for entry in entries[..len * MOVEMENT_TERRAIN_STRIDE].chunks_exact(MOVEMENT_TERRAIN_STRIDE)
        {
            terrain.push(i64::from_le_bytes(
                entry[engine_native_parts::csub_unit_definition::UNIT_ADJUSTER_ATTACK..][..8]
                    .try_into()
                    .unwrap(),
            ));
            movement_terrain.push(i64::from_le_bytes(
                entry[engine_native_parts::csub_unit_definition::UNIT_ADJUSTER_MOVEMENT..][..8]
                    .try_into()
                    .unwrap(),
            ));
        }
        first += len;
    }
    Ok(AttackStats {
        leader: handle,
        terrain,
        fort: scalar(ATTACK_FORT),
        river: scalar(ATTACK_RIVER),
        snow: scalar(ATTACK_SNOW),
        night: word(ATTACK_NIGHT)?,
        movement_terrain,
    })
}
