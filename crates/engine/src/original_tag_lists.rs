//! Stored `_CountriesForOriginalTags`, not a reconstruction from Countries.
//!
//! 1401DA010 initially appends each country-array occurrence, including slot
//! zero, to this cache. SetOriginalTag@1401EE1C0 subsequently removes/appends
//! members, so current country-array order does not prove current cache order.
//! GetCountriesForOriginalTag@1401DB900 selects a canonical bucket through
//! GetIndex@140BA6240; 1413AA900 copies its ordered raw IDs and then appends the
//! positive original raw reference unconditionally. Never resolve, filter or
//! deduplicate the stored members here. These vectors have no text-save field.

use crate::{Memory, cpdx_array::Array, read};

/// CGameState CPdxArray<CPdxArray<CountryTag>>; 1401DA010/1401DB900 index its
/// data by canonical slot * 0x18. 1401D1BC0 bounds the outer walk with +0x994.
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

/// One owned span in the member arena. Temporary native selection controls
/// serve only this capture and are cleared before returning to the caller.
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
    /// Ok proves every outer bucket and all of its members were copied.
    /// Err withholds this entire field, even if some arena bytes were written.
    pub len: Result<usize, &'static str>,
    /// Complete header demand on capacity refusal; known minima on native
    /// failure. These hints never imply recorded emptiness or availability.
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
    let (outer_header, outer) = header(memory, address, crate::cpdx_array::STRIDE)?;
    demand.buckets = outer.count as usize;
    for ordinal in 0..outer.count {
        let address = outer
            .element(ordinal, crate::cpdx_array::STRIDE)
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

/// Header-only sizing outside the callback. A failed or changed selection
/// supplies no complete demand. No member payload, game call or heap is used.
pub fn demand(memory: &impl Memory, state: usize) -> Result<Demand, &'static str> {
    let mut demand = Demand::default();
    scan(memory, state, &mut demand, |_, bucket| {
        unchanged(memory, bucket.address, bucket.header)
    })?;
    Ok(demand)
}

/// Capture into separate caller-owned arenas without allocation. All bucket
/// headers are counted before testing either arena's capacity or copying any
/// members. Neither a capacity refusal nor a fault retries a native payload.
/// The caller commits spans only on Ok and can retain unrelated naming data.
pub fn capture(
    memory: &impl Memory,
    state: usize,
    buckets: &mut [Bucket],
    members: &mut [u32],
) -> Capture {
    let mut demand = Demand::default();
    let len = (|| {
        let (outer_address, outer_header) = scan(memory, state, &mut demand, |ordinal, bucket| {
            if let Some(destination) = buckets.get_mut(ordinal) {
                *destination = bucket;
            }
            Ok(())
        })?;
        if demand.buckets > buckets.len() {
            return Err(BUCKET_CAPACITY);
        }
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
            crate::cpdx_array::copy_u32s(
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
