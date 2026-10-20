//! Army-leader extension of unit-leader state. Skills retain signed i32 values.

/// Army-leader primary vtable.
pub const VTABLE_RVA: usize = 0x0293_f538;
/// Attack skill, i32.
pub const ATTACK: usize = 0xf58;
/// Captured byte; when set, serialization retains captor and location.
pub const CAPTURED: usize = 0x1059;
/// Defense skill, i32.
pub const DEFENSE: usize = 0xf68;
/// Planning skill, i32.
pub const PLANNING: usize = 0xf78;
/// Logistics skill, i32.
pub const LOGISTICS: usize = 0xf88;
/// Army-category caster formation reference; category must be below two before resolution.
pub const FORMATION_REFERENCE: usize = 0x1048;
/// Raw captor-country word used by raid country scope.
pub const CAPTOR_COUNTRY: usize = 0x105c;
/// Deployed-HQ byte used on the required DLC path for army-category casters.
pub const DEPLOYED_HQ: usize = 0x1068;
/// Allocation extent, independent of array stride.
pub const SIZE: usize = 0x10b8;
/// Clone retaining all signed 32-bit skills.
pub const CLONE_RVA: usize = 0x00c0_2fd0;
/// Serializer retaining signed 32-bit skills.
pub const SERIALIZE_RVA: usize = 0x00c1_8ca0;

/// Terrain-adjuster vector data.
pub const MOVEMENT_TERRAIN_DATA: usize = 0x248;
/// Terrain-adjuster record stride.
pub const MOVEMENT_TERRAIN_STRIDE: usize = 0x28;
/// Terrain-adjuster movement scalar.
pub const MOVEMENT_TERRAIN_SPEED: usize = 0x20;
/// Default cached modifier block.
pub const MOVEMENT_MODIFIER: usize = 0x408;
/// Separate-HQ cached modifier block.
pub const SEPARATE_HQ_MOVEMENT_MODIFIER: usize = 0x4c8;
/// Modifier accumulator displacement used for the movement term after selecting
/// the leader's unit modifier block.
pub const MODIFIER_ACCUMULATOR: usize = 0x10;
/// The unit-block keys the movement speed and the out-of-supply penalty read for a
/// commander, ascending: army speed, armour speed, out of supply, river crossing and
/// special-forces out of supply.
pub const MOVEMENT_MODIFIER_KEYS: [u32; 6] = [
    crate::modifier_keys::ARMY_SPEED_FACTOR,
    crate::modifier_keys::ARMY_ARMOR_SPEED_FACTOR,
    crate::modifier_keys::OUT_OF_SUPPLY_FACTOR,
    crate::modifier_keys::COMBAT_WIDTH_FACTOR,
    crate::modifier_keys::RIVER_CROSSING_FACTOR,
    crate::modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
];
/// Orders-group member primary-root cache; root lookup returns it after validation.
const PRIMARY_ROOT_CACHE: usize = 0x20;
/// Dirty byte causing both orders-group member caches to rebuild.
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

/// Demand-read primary-root dependencies for one ETA acquisition pass only.
/// This is not the separate has-order predicate: own primary roots precede
/// the immediate parent's primary roots, and fallback roots are never read.
/// Prefixes extend by the original 64-pointer chunks only when demanded.
/// Collisions and capacity use the original unread suffix; cached prefixes
/// are reused only under their current address and array-header guards.
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
            return Err(crate::allocation::oom());
        }
        Ok(Self {
            metadata: crate::carmy::PresentationInputs::new(if chunks == 0 { 0 } else { roots })?,
            prefixes: crate::allocation::filled(roots, None)?,
            chunks: crate::allocation::vec(chunks)?,
            index: crate::allocation::filled(
                chunks.checked_mul(128).ok_or_else(crate::allocation::oom)?,
                0,
            )?,
            chunk_limit: chunks,
        })
    }

    #[cfg(test)]
    pub fn new(roots: usize, chunks: usize) -> Self {
        Self::try_new(roots, chunks).unwrap()
    }

    /// Keeps nothing: every root and member chunk is read. Allocates nothing.
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

    /// Bypasses the optional Memory hook, so acquisition can use its ordinary
    /// byte adapter without recursively invoking this same cache.
    pub fn resolve(
        &mut self,
        memory: &impl crate::Memory,
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
        memory: &impl crate::Memory,
        root: usize,
        address: usize,
        header: [u8; 16],
        members: crate::cpdx_array::Array,
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
                // The header itself was acquired successfully before contains.
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
            if !crate::read_slice(memory, address, &mut chunk.bytes[..chunk.len]) {
                return Err("movement assigned members unreadable");
            }
            let found = chunk.bytes[..chunk.len]
                .chunks_exact(8)
                .any(|word| u64::from_le_bytes(word.try_into().unwrap()) == member as u64);
            let ordinal = self.chunks.len();
            self.chunks.push(chunk);
            // At most 64 distinct words per chunk; fixed index capacity is
            // twice the maximum population. Neither insertion nor lookup grows.
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
    memory: &impl crate::Memory,
    members: crate::cpdx_array::Array,
    member: usize,
    mut first: u32,
) -> Result<bool, &'static str> {
    let mut bytes = [0; 512];
    while first < members.count {
        let count = (members.count - first).min(64) as usize;
        let address = members
            .element(first, 8)
            .ok_or("movement assigned members overflow")?;
        if !crate::read_slice(memory, address, &mut bytes[..count * 8]) {
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

fn primary_shared<T: crate::memory::Raw>(
    memory: &impl crate::Memory,
    pointer: usize,
    offset: usize,
    cache: &mut Option<&mut PrimaryRootCache>,
) -> Result<T, &'static str> {
    let address = movement_at(pointer, offset)?;
    match cache.as_deref_mut() {
        Some(cache) => cache.metadata.read(memory, address),
        None => crate::read(memory, address),
    }
    .ok_or("movement leader field unreadable")
}

fn movement_at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("movement leader address overflow")
}

fn movement_field<T: crate::memory::Raw>(
    memory: &impl crate::Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    crate::read(memory, movement_at(pointer, offset)?).ok_or("movement leader field unreadable")
}

/// One gathered field, consulted as `movement_field` would read it: address
/// overflow first, then the gathered value or its read failure.
fn movement_batched<T: crate::memory::Raw, const N: usize>(
    memory: &impl crate::Memory,
    batch: &crate::memory::FieldBatch<N>,
    index: usize,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    movement_at(pointer, offset)?;
    batch
        .get(memory, index)
        .ok_or("movement leader field unreadable")
}

/// The gather fields of an object at `pointer`: invalid addresses fail there.
fn movement_fields<const N: usize>(
    pointer: usize,
    fields: [(usize, usize); N],
) -> [(usize, usize); N] {
    fields.map(|(offset, len)| (pointer.checked_add(offset).unwrap_or(0), len))
}

/// The group's identity fields and its leader, gathered together; the leader
/// is consulted only after the identity passes.
fn movement_group_leader(
    memory: &impl crate::Memory,
    base: usize,
    group: usize,
) -> Result<usize, &'static str> {
    use crate::corders_group::{HANDLE, LEADER, REFERENCE_TYPE};
    let batch = crate::memory::FieldBatch::gather(
        memory,
        movement_fields(group, [(0, 8), (HANDLE, 4), (LEADER, 8)]),
    );
    let vtable = movement_batched::<u64, 3>(memory, &batch, 0, group, 0)?;
    if ![
        crate::corders_group::VTABLE_RVA,
        crate::carmy_group::VTABLE_RVA,
    ]
    .iter()
    .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
        || movement_batched::<u32, 3>(memory, &batch, 1, group, HANDLE)? != REFERENCE_TYPE
    {
        return Err("movement leader group identity mismatch");
    }
    Ok(movement_batched::<u64, 3>(memory, &batch, 2, group, LEADER)? as usize)
}

/// The leader's vtable and type are the first two fields of `batch`.
fn movement_leader_identity<const N: usize>(
    memory: &impl crate::Memory,
    base: usize,
    leader: usize,
    batch: &crate::memory::FieldBatch<N>,
) -> Result<(), &'static str> {
    if movement_batched::<u64, N>(memory, batch, 0, leader, 0)?
        != movement_at(base, VTABLE_RVA)? as u64
        || movement_batched::<u32, N>(memory, batch, 1, leader, crate::cunit_leader::TYPE)?
            != crate::cunit_leader::REFERENCE_TYPE
    {
        return Err("movement army leader identity mismatch");
    }
    Ok(())
}

/// Returns the first assigned primary root, preserving own-before-parent order.
/// Fallback roots affect the other native cache and never participate here.
fn movement_primary_root(
    memory: &impl crate::Memory,
    base: usize,
    member: usize,
) -> Result<usize, &'static str> {
    if let Some(result) = memory.movement_primary_root(base, member) {
        return result;
    }
    movement_primary_root_internal(memory, base, member, None)
}

fn movement_primary_root_internal(
    memory: &impl crate::Memory,
    base: usize,
    member: usize,
    mut cache: Option<&mut PrimaryRootCache>,
) -> Result<usize, &'static str> {
    use crate::{
        corder_instance as order, corders_group as group, corders_group_member, cpdx_array::Array,
    };
    let own = movement_field::<u64>(memory, member, corders_group_member::GROUP)? as usize;
    let dirty = movement_field::<u8>(memory, member, PRIMARY_ROOT_DIRTY)?;
    let cached = movement_field::<u64>(memory, member, PRIMARY_ROOT_CACHE)? as usize;
    let mut expected = 0;
    let authenticate =
        |pointer, cache: &mut Option<&mut PrimaryRootCache>| -> Result<(), &'static str> {
            let vtable = primary_shared::<u64>(memory, pointer, 0, cache)?;
            if ![group::VTABLE_RVA, crate::carmy_group::VTABLE_RVA]
                .iter()
                .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
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
                        != movement_at(base, order::VTABLE_RVA)? as u64
                    || primary_shared::<u64>(memory, root, order::OWNER, &mut cache)?
                        != owner as u64
                {
                    return Err("movement primary root identity mismatch");
                }
                let members_at = movement_at(root, order::ASSIGNED_MEMBER_DATA)?;
                let header = crate::read::<[u8; 16]>(memory, members_at)
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

/// Passive modifier selection. Returns the selected CModifier's accumulator
/// (`MODIFIER_ACCUMULATOR`), not its enclosing object. Dirty order roots are derived without
/// mutating caches; a stale clean cache is unavailable, not repaired.
pub fn movement_accumulator(
    memory: &impl crate::Memory,
    base: usize,
    army: usize,
) -> Result<Option<usize>, &'static str> {
    let own = movement_field::<u64>(memory, army, crate::carmy::ORDERS_GROUP)? as usize;
    movement_accumulator_internal(memory, base, army, own, || {
        movement_field::<u32>(memory, army, crate::cunit::CLASS)
    })
}

/// Selects the leader modifier from already acquired division fields. An
/// unavailable field keeps its original failure without another native read.
pub fn movement_accumulator_seeded(
    memory: &impl crate::Memory,
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
    memory: &impl crate::Memory,
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
    // The trade feature flags are read only for a class-zero division, where
    // main reads them.
    let mut alternate = false;
    if class()? == 0
        && movement_field::<u32>(
            memory,
            base,
            crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
        )? & (1 << 17)
            != 0
        && let Some(hq) = movement_hq_root(memory, base, own)?
    {
        alternate = hq != movement_own_root(memory, base, army)?;
    }
    movement_selected(leader, alternate)
}

/// The separate-HQ predicate for `unit`, as [`movement_accumulator`] applies it to a division's own
/// leader: a class-zero unit, with the deployed-HQ feature (DLC feature 0x39, runtime mask bit 17),
/// whose group has a leader and an HQ provider, and whose primary root differs from the HQ unit's.
/// Unit modifier selection then selects a leader's separate-HQ block for it. A failed read is
/// unavailable, never false.
pub fn separate_hq_order(
    memory: &impl crate::Memory,
    base: usize,
    unit: usize,
) -> Result<bool, &'static str> {
    if movement_field::<u32>(memory, unit, crate::cunit::CLASS)? != 0
        || movement_field::<u32>(
            memory,
            base,
            crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
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

/// The group's leader, zero for none, after the group's and the leader's
/// identities authenticate. The leader's identity fields are one guarded call.
fn movement_leader(
    memory: &impl crate::Memory,
    base: usize,
    group: usize,
) -> Result<usize, &'static str> {
    let leader = movement_group_leader(memory, base, group)?;
    if leader == 0 {
        return Ok(0);
    }
    let batch = crate::memory::FieldBatch::gather(
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

/// The primary root of the group's HQ provider member, None without one, after
/// the HQ host's identity authenticates (the separate-HQ predicate compares it
/// with the division's own).
fn movement_hq_root(
    memory: &impl crate::Memory,
    base: usize,
    own: usize,
) -> Result<Option<usize>, &'static str> {
    use crate::{carmy, corders_group as group, corders_group_member, cunit};
    let hq = movement_field::<u64>(memory, own, group::HQ_PROVIDER)? as usize;
    if hq == 0 {
        return Ok(None);
    }
    let host = hq
        .checked_sub(corders_group_member::HOST_OFFSET)
        .ok_or("movement HQ host underflow")?;
    if movement_field::<u64>(memory, host, 0)? != movement_at(base, carmy::VTABLE_RVA)? as u64
        || movement_field::<u64>(memory, host, carmy::SECONDARY_BASE)?
            != movement_at(base, carmy::SECONDARY_VTABLE_RVA)? as u64
        || movement_field::<u64>(memory, host, cunit::HANDLE)? == 0
        || movement_field::<u64>(memory, hq, corders_group_member::GROUP)? != own as u64
    {
        return Err("movement HQ host identity mismatch");
    }
    movement_primary_root(memory, base, hq).map(Some)
}

/// The division's own member's primary root, compared with its HQ's.
fn movement_own_root(
    memory: &impl crate::Memory,
    base: usize,
    army: usize,
) -> Result<usize, &'static str> {
    movement_primary_root(
        memory,
        base,
        movement_at(army, crate::corders_group_member::HOST_OFFSET)?,
    )
}

/// The selected CModifier's accumulator.
fn movement_selected(leader: usize, alternate: bool) -> Result<Option<usize>, &'static str> {
    let modifier = if alternate {
        SEPARATE_HQ_MOVEMENT_MODIFIER
    } else {
        MOVEMENT_MODIFIER
    };
    Ok(Some(movement_at(leader, modifier + MODIFIER_ACCUMULATOR)?))
}

/// One pass's open-addressed memo by nonzero native pointer. Entries carry the
/// pass's generation, so a new pass forgets them without clearing the table;
/// it holds at most half its slots, and a full table only computes afresh.
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
    /// Room for `keys` entries; zero allocates nothing.
    pub(crate) fn try_with_capacity(keys: usize) -> std::io::Result<Self> {
        let slots = if keys == 0 {
            0
        } else {
            keys.checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(crate::allocation::oom)?
        };
        Ok(Self {
            slots: crate::allocation::filled(slots, None)?,
            generation: 1,
            used: 0,
        })
    }

    /// Keeps nothing; allocates nothing.
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

    /// `key`'s value this pass: the kept one, else `compute`'s, kept while
    /// the table is at most half full.
    pub(crate) fn get_or(&mut self, key: usize, compute: impl FnOnce() -> V) -> V {
        self.get_or_if(key, compute, |_| true)
    }

    /// Keep only observations the caller can reuse. In particular a failed
    /// native read must not turn a one-shot fault into later refusals.
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

/// One acquisition pass's [`movement_accumulator_seeded`] reads that depend
/// only on a division's group or on no division: the group's identity and
/// leader with the leader's identity, the group's HQ provider with its host's
/// identity and primary root, and the trade feature flags. Within one
/// quiescent acquisition none of them can change, so the pass reads each once
/// for every division of the group. A division's own member fields stay its
/// own; the groups, root orders and assigned-member chunks its own primary
/// root resolves through are read once a pass ([`PrimaryRootCache`]).
pub struct LeaderMemo {
    leaders: PassTable<Result<usize, &'static str>>,
    hq: PassTable<Result<Option<usize>, &'static str>>,
    flags: Option<Result<u32, &'static str>>,
    roots: PrimaryRootCache,
}

impl LeaderMemo {
    /// Room for `groups` groups; zero allocates nothing.
    pub fn try_with_capacity(groups: usize) -> std::io::Result<Self> {
        Ok(Self {
            leaders: PassTable::try_with_capacity(groups)?,
            hq: PassTable::try_with_capacity(groups)?,
            flags: None,
            // A full cache only reads the rest afresh.
            roots: PrimaryRootCache::try_new(groups, groups.div_ceil(4))?,
        })
    }

    /// Keeps no group: every division reads afresh. Allocates nothing.
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

    /// Starts a pass: nothing a previous pass read is used again.
    pub fn clear(&mut self) {
        self.leaders.clear();
        self.hq.clear();
        self.flags = None;
        self.roots.clear();
    }
}

/// [`movement_accumulator_seeded`] over a pass's [`LeaderMemo`]: the same
/// selection and the same first failure, from the same reads in the same
/// order, except that what depends only on the group, or on no division, is
/// read once per pass.
pub fn movement_accumulator_memoized(
    memory: &impl crate::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
    memo: &mut LeaderMemo,
) -> Result<Option<usize>, &'static str> {
    movement_block_memoized(memory, base, seed, memo).map(|block| block.map(|(root, _)| root))
}

/// [`movement_accumulator_memoized`] with the block it selected: `true` for the
/// separate-HQ block, `false` for the ordinary one.
pub fn movement_block_memoized(
    memory: &impl crate::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
    memo: &mut LeaderMemo,
) -> Result<Option<(usize, bool)>, &'static str> {
    movement_at(seed.pointer, crate::carmy::ORDERS_GROUP)?;
    let own = seed
        .orders_group
        .ok_or("movement leader field unreadable")?;
    if own == 0 {
        return Ok(None);
    }
    let leader = memo
        .leaders
        .get_or(own, || movement_leader(memory, base, own))?;
    if leader == 0 {
        return Ok(None);
    }
    movement_at(seed.pointer, crate::cunit::CLASS)?;
    let class = seed.class.ok_or("movement leader field unreadable")?;
    let mut alternate = false;
    if class == 0 {
        let flags = *memo.flags.get_or_insert_with(|| {
            movement_field::<u32>(
                memory,
                base,
                crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
            )
        });
        if flags? & (1 << 17) != 0
            && let Some(hq) = memo
                .hq
                .get_or(own, || movement_hq_root(memory, base, own))?
        {
            let member = movement_at(seed.pointer, crate::corders_group_member::HOST_OFFSET)?;
            let own = match memory.movement_primary_root(base, member) {
                Some(root) => root,
                None => memo.roots.resolve(memory, base, member),
            }?;
            alternate = hq != own;
        }
    }
    movement_selected(leader, alternate).map(|root| root.map(|root| (root, alternate)))
}

/// Commander-receiver vtable slot 8 returns the army's own identity and null for a railway unit, so
/// a railway unit never selects a commander. Read keys independently from the same selected unit
/// modifier block.
pub fn current_retreat_inputs(
    memory: &impl crate::Memory,
    base: usize,
    seed: &crate::carmy::DivisionSeed,
) -> (Option<bool>, [Result<i64, &'static str>; 2]) {
    let selected = movement_accumulator_seeded(memory, base, seed);
    let values = [
        CANNOT_RETREAT_WHILE_DEFENDING,
        CANNOT_RETREAT_WHILE_ATTACKING,
    ]
    .map(|key| match selected {
        Ok(None) => Ok(0),
        Ok(Some(root)) => crate::modifier_accumulator::lookup_effective_at(memory, base, root, key)
            .map(|value| value.effective),
        Err(reason) => Err(reason),
    });
    (selected.ok().map(|root| root.is_some()), values)
}

/// Positive defending-retreat modifier values ban disengagement from the
/// defending side. The fixed layout contract authenticates the requested keys.
pub const CANNOT_RETREAT_WHILE_DEFENDING: u32 =
    crate::modifier_keys::CANNOT_RETREAT_WHILE_DEFENDING;
/// Leader modifier `cannot_retreat_while_attacking`, tested against the attacking side.
pub const CANNOT_RETREAT_WHILE_ATTACKING: u32 =
    crate::modifier_keys::CANNOT_RETREAT_WHILE_ATTACKING;
/// [`RetreatBans`] bit for [`CANNOT_RETREAT_WHILE_DEFENDING`].
pub const RETREAT_BANNED_DEFENDING: u8 = 4;
/// [`RetreatBans`] bit for [`CANNOT_RETREAT_WHILE_ATTACKING`].
pub const RETREAT_BANNED_ATTACKING: u8 = 8;

/// Retreat-ban bits of both modifier blocks selectable for a commanded unit, and
/// each block's raw [`MOVEMENT_MODIFIER_KEYS`] entries (`None` for an absent key),
/// read from the same accumulator arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetreatBans {
    pub leader: usize,
    /// From the ordinary block.
    pub ordinary: u8,
    /// From the separate-HQ block.
    pub alternate: u8,
    /// The ordinary and the separate-HQ block's movement entries.
    pub movement: [MovementEntries; 2],
}

/// One unit block's raw [`MOVEMENT_MODIFIER_KEYS`] entries.
pub type MovementEntries = [Option<i64>; MOVEMENT_MODIFIER_KEYS.len()];

impl RetreatBans {
    /// The bits of the block [`movement_accumulator_seeded`] selected.
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

/// Stored commander retreat modifiers. The army's orders group selects its
/// commander; each effective cached modifier is tested positive. A null
/// group or leader selects no commander and bans nothing.
pub fn retreat_bans(
    memory: &impl crate::Memory,
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
    let batch = crate::memory::FieldBatch::gather(
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
            if crate::modifier_accumulator::lookup_effective_at(memory, base, accumulator, key)?
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

/// The raw [`MOVEMENT_MODIFIER_KEYS`] entries of the accumulator at `accumulator`.
fn movement_entries(
    memory: &impl crate::Memory,
    accumulator: usize,
) -> Result<MovementEntries, &'static str> {
    let mut values = [crate::modifier_accumulator::Value::default(); MOVEMENT_MODIFIER_KEYS.len()];
    crate::modifier_accumulator::lookup_many_at(
        memory,
        accumulator,
        &MOVEMENT_MODIFIER_KEYS,
        &mut values,
    )?;
    Ok(values.map(|value| value.present.then_some(value.raw)))
}

/// Groups, leaders and accumulator spans copied per guarded call in [`retreat_bans_many`].
const BANS_BATCH: usize = 64;
/// Accumulator arrays copied per scattered call; longer arrays read alone.
const BANS_SPANS: usize = 16;
/// The group's vtable, handle type word and commander pointer.
const BANS_GROUP_FIELDS: [crate::memory::RecordField; 3] = [
    crate::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: crate::corders_group::HANDLE,
        destination_offset: 8,
        len: 4,
    },
    crate::memory::RecordField {
        source_offset: crate::corders_group::LEADER,
        destination_offset: 12,
        len: 8,
    },
];
const BANS_GROUP_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&BANS_GROUP_FIELDS, 20).expect("valid group layout");
/// The leader's vtable, reference type and both modifier blocks' accumulator headers.
const BANS_LEADER_FIELDS: [crate::memory::RecordField; 4] = [
    crate::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: crate::cunit_leader::TYPE,
        destination_offset: 8,
        len: 4,
    },
    crate::memory::RecordField {
        source_offset: MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
        destination_offset: 12,
        len: 16,
    },
    crate::memory::RecordField {
        source_offset: SEPARATE_HQ_MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
        destination_offset: 28,
        len: 16,
    },
];
const BANS_LEADER_BYTES: usize = 44;
const BANS_LEADER_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&BANS_LEADER_FIELDS, BANS_LEADER_BYTES)
        .expect("valid leader layout");

/// One leader whose two accumulators are still to be searched.
#[derive(Clone, Copy)]
struct BansLeader {
    group: usize,
    leader: usize,
    blocks: [crate::cpdx_array::Array; 2],
    bits: [u8; 2],
    movement: [MovementEntries; 2],
    failure: Option<&'static str>,
}

/// Retreat-ban bits and the raw movement entries from one sorted accumulator span.
/// Keys must be positive and increasing. Absent keys are zero; present values use
/// per-walk metadata.
fn bans_from_entries(
    memory: &impl crate::Memory,
    base: usize,
    entries: &[u8],
    memo: &crate::modifier_definition::DefinitionMemo,
) -> Result<(u8, MovementEntries), &'static str> {
    let mut previous = 0;
    let mut bits = 0;
    let mut movement = [None; MOVEMENT_MODIFIER_KEYS.len()];
    for entry in entries.chunks_exact(crate::modifier_entry::STRIDE) {
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
        if crate::modifier_definition::effective_memo(memory, base, key, raw, memo)? > 0 {
            bits |= bit;
        }
    }
    Ok((bits, movement))
}

/// [`retreat_bans`] for every group of one walk, `output[i]` for `groups[i]`
/// (nonzero). Group fields, then leader fields and both accumulator headers,
/// are gathered 64 objects per guarded call, and whole accumulator arrays up
/// to 64 entries 16 per scattered call; the metadata is read once per key per
/// walk. A failed or mismatched read fails only its own group, with the error
/// [`retreat_bans`] gives it; later objects continue from the next read, and
/// no failed read is repeated. A commander whose bans `held` returns (an
/// earlier read no observed writer of either block has touched since) is not
/// read again.
pub fn retreat_bans_many(
    memory: &impl crate::Memory,
    base: usize,
    groups: &[usize],
    output: &mut [Result<Option<RetreatBans>, &'static str>],
    memo: &crate::modifier_definition::DefinitionMemo,
    held: &mut dyn FnMut(usize) -> Option<RetreatBans>,
) {
    use crate::{cpdx_array::Array, memory::RecordSources, modifier_entry::STRIDE};
    let expected = |rva: usize| base.checked_add(rva).map(|value| value as u64);
    let group_vtables = [
        expected(crate::corders_group::VTABLE_RVA),
        expected(crate::carmy_group::VTABLE_RVA),
    ];
    let leader_vtable = expected(VTABLE_RVA);
    let mut group_bytes = [0_u8; BANS_BATCH * 20];
    let mut leader_bytes = [0_u8; BANS_BATCH * BANS_LEADER_BYTES];
    let mut entry_bytes = [0_u8; BANS_SPANS * BANS_BATCH * STRIDE];
    let mut start = 0;
    while start < groups.len() {
        let chunk = &groups[start..(start + BANS_BATCH).min(groups.len())];
        let completed = memory.copy_records(
            RecordSources::Pointers(chunk),
            &BANS_GROUP_LAYOUT,
            &mut group_bytes[..chunk.len() * 20],
        );
        let read = (completed / BANS_GROUP_FIELDS.len()).min(chunk.len());
        let mut leaders = [None::<BansLeader>; BANS_BATCH];
        let mut pending = 0;
        for (index, record) in group_bytes[..read * 20].chunks_exact(20).enumerate() {
            let slot = start + index;
            let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
            if !group_vtables.contains(&Some(vtable))
                || u32::from_le_bytes(record[8..12].try_into().unwrap())
                    != crate::corders_group::REFERENCE_TYPE
            {
                output[slot] = Err("movement leader group identity mismatch");
                continue;
            }
            let leader = u64::from_le_bytes(record[12..20].try_into().unwrap()) as usize;
            if leader == 0 {
                output[slot] = Ok(None);
                continue;
            }
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
        if read < chunk.len() {
            // A group whose fields stopped short; the next read starts after it.
            output[start + read] = Err("movement leader field unreadable");
        }
        start += (read + 1).min(chunk.len());
        // Leader fields for this batch's commanders, continuing past a short read.
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
                // A short identity read is a field fault; a short header an accumulator one.
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
        // Both accumulators of every authenticated leader, in leader then block order.
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
                    // A long accumulator keeps the chunked census.
                    let mut values = [crate::modifier_accumulator::Value::default(); 2];
                    let key = [
                        CANNOT_RETREAT_WHILE_DEFENDING,
                        CANNOT_RETREAT_WHILE_ATTACKING,
                    ];
                    let offset = [MOVEMENT_MODIFIER, SEPARATE_HQ_MOVEMENT_MODIFIER][block];
                    let result = movement_at(leader.leader, offset + MODIFIER_ACCUMULATOR)
                        .and_then(|at| {
                            crate::modifier_accumulator::lookup_many_at(
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
                                    && crate::modifier_definition::effective_memo(
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

/// Terrain selection followed by the signed speed read.
/// Invalid indices select entry zero, including a zero live count when its
/// retained allocation remains readable, exactly as the native fallback does.
pub fn movement_terrain(
    memory: &impl crate::Memory,
    base: usize,
    leader: usize,
    terrain_index: i32,
) -> Result<i64, &'static str> {
    let batch = crate::memory::FieldBatch::gather(
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
        .and_then(|header| crate::cpdx_array::Array::from_header(header, MOVEMENT_TERRAIN_STRIDE))
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

/// The unit leader's night adjuster's attack, added to the night term. Trait recalculation clears
/// and rebuilds it with the terrain adjusters.
pub const ATTACK_NIGHT: usize = 0x270;
/// The army leader's fort, river and snow adjusters' attack contributions. Army trait recalculation
/// rebuilds them.
pub const ATTACK_FORT: usize = 0xfb8;
pub const ATTACK_RIVER: usize = 0xfe0;
pub const ATTACK_SNOW: usize = 0x1030;

/// A direct commander's attack operands: its full handle, the attack of each
/// terrain adjuster (`MOVEMENT_TERRAIN_DATA`) in order, and its fort, river,
/// snow and night values, each an exact fixed5 word.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttackStats {
    pub leader: u64,
    pub terrain: Vec<i64>,
    pub fort: i64,
    pub river: i64,
    pub snow: i64,
    pub night: i64,
    /// Each terrain adjuster's movement, parallel to `terrain`: the movement speed adds the
    /// destination terrain's entry on every edge.
    pub movement_terrain: Vec<i64>,
}

/// Reads [`AttackStats`] of the `CUnitLeader` an orders group's direct-commander getter names.
/// Terrain-adjuster lookup returns the vector's first record for an index outside its live count,
/// even a count of zero, so a vector with storage and no live entry yields that one retained
/// record; a vector without storage yields none.
pub fn attack_stats(
    memory: &impl crate::Memory,
    base: usize,
    leader: usize,
) -> Result<AttackStats, &'static str> {
    let handle = crate::cunit_leader::handle(memory, base, leader)?;
    let word = |offset: usize| -> Result<i64, &'static str> {
        leader
            .checked_add(offset)
            .and_then(|address| crate::read::<i64>(memory, address))
            .ok_or("commander attack operand unreadable")
    };
    let mut scalars = [0_u8; ATTACK_SNOW + 8 - ATTACK_FORT];
    if !crate::read_slice(
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
    let header = crate::read::<[u8; 16]>(
        memory,
        leader
            .checked_add(MOVEMENT_TERRAIN_DATA)
            .ok_or("commander address overflow")?,
    )
    .ok_or("commander terrain adjuster header unreadable")?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let count = i32::from_le_bytes(header[12..16].try_into().unwrap());
    // The live count; a nonpositive one still leaves the first record.
    let records = if data == 0 {
        0
    } else {
        usize::try_from(count).unwrap_or(0).max(1)
    };
    let mut terrain = Vec::new();
    crate::allocation::reserve_exact(&mut terrain, records)
        .map_err(|_| "commander terrain allocation failed")?;
    let mut movement_terrain = Vec::new();
    crate::allocation::reserve_exact(&mut movement_terrain, records)
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
        if !crate::read_slice(
            memory,
            address,
            &mut entries[..len * MOVEMENT_TERRAIN_STRIDE],
        ) {
            return Err("commander terrain adjusters unreadable");
        }
        for entry in entries[..len * MOVEMENT_TERRAIN_STRIDE].chunks_exact(MOVEMENT_TERRAIN_STRIDE)
        {
            terrain.push(i64::from_le_bytes(
                entry[crate::csub_unit_definition::UNIT_ADJUSTER_ATTACK..][..8]
                    .try_into()
                    .unwrap(),
            ));
            movement_terrain.push(i64::from_le_bytes(
                entry[crate::csub_unit_definition::UNIT_ADJUSTER_MOVEMENT..][..8]
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
