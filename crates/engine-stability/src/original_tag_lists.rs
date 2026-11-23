
use engine_native_memory::{Memory, cpdx_array::Array, read};

pub const BUCKETS: usize = 0x988;
pub const BUCKET_CAPACITY: &str = "original-tag bucket arena capacity exceeded";
pub const MEMBER_CAPACITY: &str = "original-tag member arena capacity exceeded";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub buckets: usize,
    pub members: usize,
}

impl Demand {
    pub fn max(self, other: Self) -> Self {
        Self {
            buckets: self.buckets.max(other.buckets),
            members: self.members.max(other.members),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub start: usize,
    pub len: usize,
    source: Array,
    address: usize,
    header: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub len: Result<usize, &'static str>,
    pub demand: Demand,
}

fn header(
    memory: &impl Memory,
    address: usize,
    stride: usize,
) -> Result<([u8; 16], Array), &'static str> {
    let bytes = read::<[u8; 16]>(memory, address).ok_or("original-tag array header unreadable")?;
    let array = Array::from_header(bytes, stride).ok_or("original-tag array header invalid")?;
    Ok((bytes, array))
}

fn unchanged(memory: &impl Memory, address: usize, expected: [u8; 16]) -> Result<(), &'static str> {
    if read::<[u8; 16]>(memory, address) == Some(expected) {
        Ok(())
    } else {
        Err("original-tag array header changed or unreadable")
    }
}

fn scan(
    memory: &impl Memory,
    state: usize,
    demand: &mut Demand,
    mut each: impl FnMut(usize, Bucket) -> Result<(), &'static str>,
) -> Result<(usize, [u8; 16]), &'static str> {
    let address = state
        .checked_add(BUCKETS)
        .filter(|_| state != 0)
        .ok_or("original-tag game state null or overflowed")?;
    let (outer_header, outer) = header(memory, address, engine_native_memory::cpdx_array::STRIDE)?;
    demand.buckets = outer.count as usize;
    for ordinal in 0..outer.count {
        let address = outer
            .element(ordinal, engine_native_memory::cpdx_array::STRIDE)
            .ok_or("original-tag bucket address overflow")?;
        let (header, source) = header(memory, address, 4)?;
        let start = demand.members;
        demand.members = start
            .checked_add(source.count as usize)
            .ok_or("original-tag member demand overflow")?;
        each(
            ordinal as usize,
            Bucket {
                start,
                len: source.count as usize,
                source,
                address,
                header,
            },
        )?;
    }
    unchanged(memory, address, outer_header)?;
    Ok((address, outer_header))
}

pub fn demand(memory: &impl Memory, state: usize) -> Result<Demand, &'static str> {
    let mut demand = Demand::default();
    scan(memory, state, &mut demand, |_, bucket| {
        unchanged(memory, bucket.address, bucket.header)
    })?;
    Ok(demand)
}

pub fn capture(
    memory: &impl Memory,
    state: usize,
    buckets: &mut Vec<Bucket>,
    members: &mut Vec<u32>,
) -> Capture {
    capture_buffer(
        memory,
        state,
        engine_native_memory::walk::Buffer::Growing(buckets),
        engine_native_memory::walk::Buffer::Growing(members),
    )
}

fn capture_buffer(
    memory: &impl Memory,
    state: usize,
    mut buckets: engine_native_memory::walk::Buffer<'_, Bucket>,
    mut members: engine_native_memory::walk::Buffer<'_, u32>,
) -> Capture {
    let mut demand = Demand::default();
    let len = (|| {
        let (outer_address, outer_header) = scan(memory, state, &mut demand, |ordinal, bucket| {
            buckets.ensure(ordinal + 1)?;
            if let Some(destination) = buckets.get_mut(ordinal) {
                *destination = bucket;
            }
            Ok(())
        })?;
        if demand.buckets > buckets.len() {
            return Err(BUCKET_CAPACITY);
        }
        members.ensure(demand.members)?;
        if demand.members > members.len() {
            return Err(MEMBER_CAPACITY);
        }
        for bucket in &buckets[..demand.buckets] {
            let end = bucket
                .start
                .checked_add(bucket.len)
                .ok_or("original-tag member span overflow")?;
            let output = members
                .get_mut(bucket.start..end)
                .ok_or("original-tag member span outside arena")?;
            engine_native_memory::cpdx_array::copy_u32s(
                memory,
                bucket.source,
                output,
                "original-tag member payload unreadable",
            )?;
            unchanged(memory, bucket.address, bucket.header)?;
        }
        unchanged(memory, outer_address, outer_header)?;
        Ok(demand.buckets)
    })();
    for bucket in buckets.iter_mut().take(demand.buckets) {
        bucket.source = Array::default();
        bucket.address = 0;
        bucket.header = [0; 16];
    }
    Capture { len, demand }
}

#[cfg(test)]
#[path = "original_tag_lists/tests.rs"]
mod tests;
