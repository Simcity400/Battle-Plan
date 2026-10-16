//! CFrontSection: the geometry belongs to a section, not to CFront itself.
use crate::{
    Memory,
    cpdx_array::Array,
    cprovince,
    memory::{
        Gathered, RecordField, RecordLayout, gather_records, packed_fields, packed_map,
        packed_stride,
    },
    read, read_slice,
};

/// Front-section primary vtable.
pub const VTABLE_RVA: usize = 0x0295_eec0;
/// Front-section id used for section lookup.
pub const ID: usize = 8;
/// Owning front pointer.
pub const FRONT: usize = 0x10;
/// Province-pair array with 16-byte entries, each holding two province pointers.
pub const PAIRS: usize = 0x48;
/// Signed province-pair count; the word before it holds the capacity.
pub const PAIR_COUNT: usize = 0x54;

/// Pairs per guarded copy of the pair array, and per gather of their
/// provinces.
const PAIR_BATCH: usize = 64;
/// What each pair's province contributes: its identity and its id, the two
/// reads the per-pair walk makes, gathered in one record per province.
const PROVINCE_SPANS: [(usize, usize); 2] = [(0, 8), (cprovince::ID, 4)];
const PROVINCE_FIELDS: [RecordField; 2] = packed_fields(PROVINCE_SPANS);
const PROVINCE_STRIDE: usize = packed_stride(PROVINCE_SPANS);
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, PROVINCE_STRIDE).expect("front province layout");
const PROVINCE_MAP: [u16; cprovince::ID + 4] = packed_map(PROVINCE_SPANS);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub pair_start: usize,
    pub pair_len: usize,
}

pub(crate) fn header(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    front: usize,
) -> Result<(u32, Array), &'static str> {
    if read::<u64>(memory, pointer) != base.checked_add(VTABLE_RVA).map(|v| v as u64)
        || crate::corder_instance::plan_field::<u64>(memory, pointer, FRONT)? != front as u64
    {
        return Err("front section identity mismatch");
    }
    let id = crate::corder_instance::plan_field(memory, pointer, ID)?;
    let pairs = Array::read(
        memory,
        pointer.checked_add(PAIRS).ok_or("front section overflow")?,
        16,
    )
    .ok_or("front pair header invalid")?;
    Ok((id, pairs))
}

/// Appends each pair's two province ids at `start` in `output`, after
/// authenticating both provinces. Pairs are copied up to [`PAIR_BATCH`] per
/// guarded copy and their provinces gathered one record each; a pair whose
/// batch did not copy, or a province whose record did not complete, makes
/// the per-pair walk's own reads, so ids, the output prefix and the first
/// failure are those of the per-pair walk. Memory cannot change while the
/// pass holds the game thread, so a gathered byte is the byte that read
/// copies.
pub(crate) fn copy_pairs(
    memory: &impl Memory,
    base: usize,
    pairs: Array,
    output: &mut Vec<u32>,
    start: usize,
) -> Result<(), &'static str> {
    let vtable = base.checked_add(cprovince::VTABLE_RVA).map(|v| v as u64);
    let mut batch = [0_u8; PAIR_BATCH * 16];
    let mut provinces = [0_usize; PAIR_BATCH * 2];
    let mut records = [0_u8; PAIR_BATCH * 2 * PROVINCE_STRIDE];
    let mut complete = [false; PAIR_BATCH * 2];
    for first in (0..pairs.count).step_by(PAIR_BATCH) {
        let count = (pairs.count - first).min(PAIR_BATCH as u32) as usize;
        let copied = pairs
            .element(first, 16)
            .is_some_and(|address| read_slice(memory, address, &mut batch[..count * 16]));
        if copied {
            for (province, bytes) in provinces
                .iter_mut()
                .zip(batch[..count * 16].chunks_exact(8))
            {
                *province = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            }
            gather_records(
                memory,
                &provinces[..count * 2],
                &PROVINCE_LAYOUT,
                &mut records[..count * 2 * PROVINCE_STRIDE],
                &mut complete[..count * 2],
            );
        }
        for index in 0..count {
            let i = first + index as u32;
            let pointers: [u8; 16] = if copied {
                batch[index * 16..(index + 1) * 16].try_into().unwrap()
            } else {
                read::<[u8; 16]>(memory, pairs.element(i, 16).ok_or("front pair overflow")?)
                    .ok_or("front pair unreadable")?
            };
            let mut pair = [0; 2];
            for (side, (word, bytes)) in pair.iter_mut().zip(pointers.chunks_exact(8)).enumerate() {
                let pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
                let record = if copied {
                    Gathered::row(
                        &PROVINCE_LAYOUT,
                        &PROVINCE_MAP,
                        &records,
                        &complete,
                        index * 2 + side,
                    )
                } else {
                    Gathered::NONE
                };
                if pointer == 0
                    || record
                        .get::<u64>(0)
                        .or_else(|| read::<u64>(memory, pointer))
                        != vtable
                {
                    return Err("front province identity mismatch");
                }
                *word =
                    crate::corder_instance::gathered_field(memory, pointer, cprovince::ID, record)?;
                if *word == 0 {
                    return Err("front province sentinel");
                }
            }
            let end = start
                .checked_add((i as usize + 1) * 2)
                .ok_or("front pair length overflow")?;
            if end > output.len() {
                crate::allocation::reserve(output, end - output.len())
                    .map_err(|_| "front pair allocation failed")?;
                output.resize(end, 0);
            }
            output[end - 2..end].copy_from_slice(&pair);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
