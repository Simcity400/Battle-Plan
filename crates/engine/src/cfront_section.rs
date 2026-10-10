//! CFrontSection: the geometry belongs to a section, not to CFront itself.
use crate::{Memory, cpdx_array::Array, cprovince, read};

/// Front-section primary vtable.
pub const VTABLE_RVA: usize = 0x0295_eec0;
/// Front-section id used for section lookup.
pub const ID: usize = 8;
/// Owning front pointer.
pub const FRONT: usize = 0x10;
/// Province-pair array with 16-byte entries, each holding two province pointers.
pub const PAIRS: usize = 0x48;
/// Signed province-pair count; +0x50 holds capacity.
pub const PAIR_COUNT: usize = 0x54;

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

pub(crate) fn copy_pairs(
    memory: &impl Memory,
    base: usize,
    pairs: Array,
    output: &mut Vec<u32>,
    start: usize,
) -> Result<(), &'static str> {
    for i in 0..pairs.count {
        let mut pair = [0; 2];
        let pointers = read::<[u8; 16]>(memory, pairs.element(i, 16).ok_or("front pair overflow")?)
            .ok_or("front pair unreadable")?;
        for (word, bytes) in pair.iter_mut().zip(pointers.chunks_exact(8)) {
            let pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            if pointer == 0
                || read::<u64>(memory, pointer)
                    != base.checked_add(cprovince::VTABLE_RVA).map(|v| v as u64)
            {
                return Err("front province identity mismatch");
            }
            *word = crate::corder_instance::plan_field(memory, pointer, cprovince::ID)?;
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
    Ok(())
}
