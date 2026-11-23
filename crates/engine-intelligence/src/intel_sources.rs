
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, read, read_slice};
use engine_allocation as allocation;

pub const ALLIED: usize = 0x28;
pub const STATIC_CONTRIBUTIONS: usize = 0x40;
pub const DYNAMIC_CONTRIBUTIONS: usize = 0x58;
pub const PRIOR_CONTRIBUTIONS: usize = 0x70;
pub const CRYPTOLOGY: usize = 0x88;
pub const STATIC_POOLS: usize = 0xa0;
pub const DYNAMIC_POOLS: usize = 0xb8;
pub const LIFETIME: usize = 0xd0;
pub const CHEAT_REFERENCE: usize = 0xd8;
pub const RADAR_REFERENCE: usize = 0x48;
pub const TOKEN_REFERENCE: usize = 0x10;
pub const STATIC_POOL_STRIDE: usize = 0x48;
pub const DYNAMIC_POOL_STRIDE: usize = 0x38;
pub const STATIC_POOL_VTABLE: Rva = Rva(0x029d_ba88);
pub const DYNAMIC_POOL_VTABLE: Rva = Rva(0x029d_b9e8);
pub const SOURCE_VTABLE: Rva = Rva(0x029d_ba38);
pub const REFERENCE_VTABLE: Rva = Rva(0x029d_bad8);
pub const POOL_SOURCES: usize = 0x28;
pub const NEXT_ID: usize = 0x40;
pub const SOURCE_STRIDE: usize = 0x28;
pub const SOURCE_VALUES: usize = 8;
pub const SOURCE_ID: usize = 0x20;
pub const SOURCE_REMOVED: usize = 0x24;
pub const DYNAMIC_VALUES: usize = 8;
pub const DYNAMIC_ACCUMULATORS: usize = 0x20;
pub const KEYED_STRIDE: usize = 0x28;
pub const DENSE_STRIDE: usize = 0x20;
pub const OUTER_STRIDE: usize = 0x18;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceReference {
    pub country: u32,
    pub pool: i32,
    pub id: i32,
    pub lifetime: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lanes {
    pub target: i32,
    pub values: [i64; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DenseEntry {
    pub ordinal: u32,
    pub values: [i64; 4],
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dense {
    pub count: u32,
    pub entries: Vec<DenseEntry>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticSource {
    pub id: i32,
    pub removed: u8,
    pub entries: Vec<Lanes>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticPool {
    pub next_id: i32,
    pub sources: Vec<StaticSource>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicPool {
    pub values: Vec<Lanes>,
    pub accumulators: Vec<Lanes>,
}

pub fn set_source(
    source: &mut StaticSource,
    target: i32,
    values: [i64; 4],
) -> Result<bool, &'static str> {
    let index = lower_bound(&source.entries, target);
    if let Some(entry) = source
        .entries
        .get_mut(index)
        .filter(|entry| entry.target == target)
    {
        let changed = entry.values != values;
        entry.values = values;
        return Ok(changed);
    }
    if values == [0; 4] {
        return Ok(false);
    }
    allocation::reserve(&mut source.entries, 1)
        .map_err(|_| "intel derived source insertion allocation failed")?;
    source.entries.insert(index, Lanes { target, values });
    Ok(true)
}

pub fn add_pending(
    pool: &mut DynamicPool,
    target: i32,
    values: [i64; 4],
) -> Result<bool, &'static str> {
    let values = values.map(|value| value.max(0));
    if values == [0; 4] {
        return Ok(false);
    }
    let index = lower_bound(&pool.values, target);
    if pool
        .values
        .get(index)
        .is_none_or(|entry| entry.target != target)
    {
        allocation::reserve(&mut pool.values, 1)
            .map_err(|_| "intel derived pending insertion allocation failed")?;
        pool.values.insert(
            index,
            Lanes {
                target,
                values: [0; 4],
            },
        );
    }
    let entry = &mut pool.values[index];
    let old = entry.values;
    entry.values = std::array::from_fn(|lane| entry.values[lane].wrapping_add(values[lane]));
    Ok(entry.values != old)
}

pub fn prune_sources(pool: &mut StaticPool) -> bool {
    let mut changed = false;
    for index in (0..pool.sources.len()).rev() {
        if pool.sources[index].removed != 0 {
            pool.sources.swap_remove(index);
            changed = true;
        }
    }
    changed
}

pub fn decay_accumulators(pool: &mut DynamicPool, subtraction: i64, factor: i64) -> bool {
    let mut changed = false;
    for entry in &mut pool.accumulators {
        for value in &mut entry.values {
            let next = (value.wrapping_sub(subtraction).wrapping_mul(factor) / 100_000).max(0);
            changed |= *value != next;
            *value = next;
        }
    }
    changed
}

fn lower_bound(entries: &[Lanes], target: i32) -> usize {
    let (mut begin, mut count) = (0, entries.len());
    while count != 0 {
        let half = count / 2;
        if entries[begin + half].target < target {
            begin += count - half;
        }
        count = half;
    }
    begin
}

pub fn reference(
    memory: &impl Memory,
    base: usize,
    object: usize,
) -> Result<SourceReference, &'static str> {
    authenticate(memory, base, object, REFERENCE_VTABLE)?;
    let bytes = field::<[u8; 16]>(memory, object, 8)?;
    let values: [u32; 4] = std::array::from_fn(|index| {
        u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    });
    Ok(SourceReference {
        country: values[0],
        pool: values[1] as i32,
        id: values[2] as i32,
        lifetime: values[3],
    })
}

pub fn dense(
    memory: &impl Memory,
    root: usize,
    offset: usize,
    output: &mut Dense,
) -> Result<(), &'static str> {
    let array = array(memory, root, offset, DENSE_STRIDE)?;
    output.count = array.count;
    output.entries.clear();
    const BATCH: usize = 64;
    let mut bytes = [0_u8; BATCH * DENSE_STRIDE];
    let mut ordinal = 0;
    while ordinal < array.count {
        let count = (array.count - ordinal).min(BATCH as u32) as usize;
        let at = array
            .element(ordinal, DENSE_STRIDE)
            .ok_or("intel dense entry address overflow")?;
        let span = &mut bytes[..count * DENSE_STRIDE];
        if !read_slice(memory, at, span) {
            return Err("intel dense source lanes unreadable");
        }
        for (index, entry) in span.chunks_exact(DENSE_STRIDE).enumerate() {
            let values = std::array::from_fn(|lane| {
                i64::from_le_bytes(entry[lane * 8..lane * 8 + 8].try_into().unwrap())
            });
            if values != [0; 4] {
                allocation::reserve(&mut output.entries, 1)
                    .map_err(|_| "intel dense source storage allocation failed")?;
                output.entries.push(DenseEntry {
                    ordinal: ordinal + index as u32,
                    values,
                });
            }
        }
        ordinal += count as u32;
    }
    Ok(())
}

pub fn keyed(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    output: &mut Vec<Lanes>,
) -> Result<(), &'static str> {
    let array = array(memory, object, offset, KEYED_STRIDE)?;
    allocation::resize_default(output, array.count as usize)
        .map_err(|_| "intel keyed source storage allocation failed")?;
    for ordinal in 0..array.count {
        let at = array
            .element(ordinal, KEYED_STRIDE)
            .ok_or("intel keyed entry address overflow")?;
        output[ordinal as usize] = Lanes {
            target: field::<i32>(memory, at, 0)?,
            values: lane_values(
                memory,
                at.checked_add(8)
                    .ok_or("intel keyed lane address overflow")?,
            )?,
        };
    }
    Ok(())
}

pub fn lane_values(memory: &impl Memory, address: usize) -> Result<[i64; 4], &'static str> {
    let bytes = read::<[u8; 32]>(memory, address).ok_or("intel source lanes unreadable")?;
    Ok(std::array::from_fn(|lane| {
        i64::from_le_bytes(bytes[lane * 8..lane * 8 + 8].try_into().unwrap())
    }))
}

pub fn contributions(
    memory: &impl Memory,
    root: usize,
    offset: usize,
    output: &mut Vec<Vec<Lanes>>,
) -> Result<(), &'static str> {
    let pools = array(memory, root, offset, OUTER_STRIDE)?;
    allocation::resize_default(output, pools.count as usize)
        .map_err(|_| "intel contribution pool storage allocation failed")?;
    for ordinal in 0..pools.count {
        let at = pools
            .element(ordinal, OUTER_STRIDE)
            .ok_or("intel contribution pool address overflow")?;
        keyed(memory, at, 0, &mut output[ordinal as usize])?;
    }
    Ok(())
}

pub fn static_source(
    memory: &impl Memory,
    base: usize,
    object: usize,
    output: &mut StaticSource,
) -> Result<(), &'static str> {
    authenticate(memory, base, object, SOURCE_VTABLE)?;
    output.id = field(memory, object, SOURCE_ID)?;
    output.removed = field(memory, object, SOURCE_REMOVED)?;
    keyed(memory, object, SOURCE_VALUES, &mut output.entries)
}

pub fn static_pool(
    memory: &impl Memory,
    base: usize,
    object: usize,
    output: &mut StaticPool,
) -> Result<(), &'static str> {
    authenticate(memory, base, object, STATIC_POOL_VTABLE)?;
    output.next_id = field(memory, object, NEXT_ID)?;
    let sources = array(memory, object, POOL_SOURCES, SOURCE_STRIDE)?;
    allocation::resize_default(&mut output.sources, sources.count as usize)
        .map_err(|_| "intel static source storage allocation failed")?;
    for ordinal in 0..sources.count {
        let at = sources
            .element(ordinal, SOURCE_STRIDE)
            .ok_or("intel source address overflow")?;
        static_source(memory, base, at, &mut output.sources[ordinal as usize])?;
    }
    Ok(())
}

pub fn dynamic_pool(
    memory: &impl Memory,
    base: usize,
    object: usize,
    output: &mut DynamicPool,
) -> Result<(), &'static str> {
    authenticate(memory, base, object, DYNAMIC_POOL_VTABLE)?;
    keyed(memory, object, DYNAMIC_VALUES, &mut output.values)?;
    keyed(
        memory,
        object,
        DYNAMIC_ACCUMULATORS,
        &mut output.accumulators,
    )
}

pub fn static_pools(
    memory: &impl Memory,
    base: usize,
    root: usize,
    output: &mut Vec<StaticPool>,
) -> Result<(), &'static str> {
    let pools = array(memory, root, STATIC_POOLS, STATIC_POOL_STRIDE)?;
    allocation::resize_default(output, pools.count as usize)
        .map_err(|_| "intel static pool storage allocation failed")?;
    for ordinal in 0..pools.count {
        static_pool(
            memory,
            base,
            pools
                .element(ordinal, STATIC_POOL_STRIDE)
                .ok_or("intel static pool address overflow")?,
            &mut output[ordinal as usize],
        )?;
    }
    Ok(())
}
pub fn dynamic_pools(
    memory: &impl Memory,
    base: usize,
    root: usize,
    output: &mut Vec<DynamicPool>,
) -> Result<(), &'static str> {
    let pools = array(memory, root, DYNAMIC_POOLS, DYNAMIC_POOL_STRIDE)?;
    allocation::resize_default(output, pools.count as usize)
        .map_err(|_| "intel dynamic pool storage allocation failed")?;
    for ordinal in 0..pools.count {
        dynamic_pool(
            memory,
            base,
            pools
                .element(ordinal, DYNAMIC_POOL_STRIDE)
                .ok_or("intel dynamic pool address overflow")?,
            &mut output[ordinal as usize],
        )?;
    }
    Ok(())
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    let vtable = engine_sites::at(base, vtable).ok_or("intel source object identity mismatch")?;
    if object == 0 || read::<u64>(memory, object) != Some(vtable as u64) {
        Err("intel source object identity mismatch")
    } else {
        Ok(())
    }
}
pub fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("intel source array header invalid")
}
fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("intel source operand unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::ccountry::synthetic::World;
    #[test]
    fn source_mutations_derive_keyed_zero_wrapping_addition_and_native_prune_order() {
        let mut source = StaticSource::default();
        assert!(!set_source(&mut source, -7, [0; 4]).unwrap());
        assert!(set_source(&mut source, -7, [1, 2, 3, 4]).unwrap());
        assert!(set_source(&mut source, -7, [0; 4]).unwrap());
        assert_eq!(
            source.entries,
            [Lanes {
                target: -7,
                values: [0; 4]
            }]
        );
        let mut pool = DynamicPool {
            values: vec![Lanes {
                target: 42,
                values: [i64::MAX, 0, 0, 0],
            }],
            ..Default::default()
        };
        assert!(add_pending(&mut pool, 42, [1, -5, 0, 0]).unwrap());
        assert_eq!(pool.values[0].values, [i64::MIN, 0, 0, 0]);
        assert!(!add_pending(&mut pool, -1, [-1, -2, 0, 0]).unwrap());
        assert_eq!(pool.values.len(), 1);
        pool.accumulators = vec![Lanes {
            target: -17,
            values: [300_000, 100_000, -7, i64::MIN],
        }];
        assert!(decay_accumulators(&mut pool, 100_000, 50_000));
        assert_eq!(pool.accumulators[0].values, [100_000, 0, 0, 0]);
        let mut pool = StaticPool {
            sources: (0..5)
                .map(|id| StaticSource {
                    id,
                    removed: u8::from(id == 1 || id == 3),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert!(prune_sources(&mut pool));
        assert_eq!(
            pool.sources
                .iter()
                .map(|source| source.id)
                .collect::<Vec<_>>(),
            [0, 4, 2]
        );
        assert!(!prune_sources(&mut pool));
    }

    #[test]
    fn source_contributions_preserve_pool_extents_signed_keys_duplicates_and_keyed_zero() {
        let mut world = World::new(0, 1, 0x200);
        let root = world.alloc(0xf0);
        let pools = world.alloc(2 * OUTER_STRIDE);
        let entries = world.alloc(3 * KEYED_STRIDE);
        world.u64(root + STATIC_CONTRIBUTIONS, pools as u64);
        world.u32(root + STATIC_CONTRIBUTIONS + 8, 2);
        world.u32(root + STATIC_CONTRIBUTIONS + 12, 2);
        world.u64(pools, entries as u64);
        world.u32(pools + 8, 3);
        world.u32(pools + 12, 3);
        for ordinal in 0..3 {
            world.u32(entries + ordinal * KEYED_STRIDE, (-17_i32) as u32);
            world.u64(
                entries + ordinal * KEYED_STRIDE + 8,
                if ordinal == 1 { i64::MIN as u64 } else { 0 },
            );
        }
        let mut output = Vec::new();
        contributions(&world.memory(), root, STATIC_CONTRIBUTIONS, &mut output).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].len(), 3);
        assert_eq!(
            output[0][1],
            Lanes {
                target: -17,
                values: [i64::MIN, 0, 0, 0]
            }
        );
        assert_eq!(output[0][0], output[0][2]);
        assert!(output[1].is_empty());
    }
    #[test]
    fn dense_source_zeros_keep_live_extent_without_materializing_zero_pairs() {
        let mut world = World::new(0, 1, 0x200);
        let root = world.alloc(0xf0);
        let entries = world.alloc(5000 * DENSE_STRIDE);
        world.u64(root + ALLIED, entries as u64);
        world.u32(root + ALLIED + 8, 5000);
        world.u32(root + ALLIED + 12, 5000);
        world.u64(entries + 4998 * DENSE_STRIDE + 24, (-4_i64) as u64);
        let mut output = Dense::default();
        dense(&world.memory(), root, ALLIED, &mut output).unwrap();
        assert_eq!(output.count, 5000);
        assert_eq!(
            output.entries,
            [DenseEntry {
                ordinal: 4998,
                values: [0, 0, 0, -4]
            }]
        );
    }
}
