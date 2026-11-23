
pub const STRIDE: usize = 0x28;
pub const HASH: usize = 0x00;
pub const OCCUPIED: usize = 0x04;
pub const TARGET_TAG_ID: usize = 0x08;
pub const TOKENS: usize = 0x10;
pub const TOKEN_CAPACITY: usize = 0x18;
pub const TOKEN_COUNT: usize = 0x1c;
pub const TOKEN_STRIDE: usize = 0x04;

use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice};

pub(crate) struct Entry {
    pub hash: u32,
    pub distance: u8,
    pub target: u32,
    pub tokens: Array,
}

pub(crate) fn read_entry(
    memory: &impl Memory,
    address: usize,
) -> Result<Option<Entry>, &'static str> {
    let bytes = read::<[u8; 32]>(memory, address).ok_or("operation token map slot unreadable")?;
    let occupied = bytes[OCCUPIED];
    match occupied {
        0 => return Ok(None),
        0xff => return Err("operation token map sentinel inside census"),
        _ => {}
    }
    let target = u32::from_le_bytes(bytes[TARGET_TAG_ID..TARGET_TAG_ID + 4].try_into().unwrap());
    let data = usize::try_from(u64::from_le_bytes(
        bytes[TOKENS..TOKEN_CAPACITY].try_into().unwrap(),
    ))
    .map_err(|_| "operation token vector pointer overflow")?;
    let capacity = i32::from_le_bytes(bytes[TOKEN_CAPACITY..TOKEN_COUNT].try_into().unwrap());
    let count = i32::from_le_bytes(bytes[TOKEN_COUNT..32].try_into().unwrap());
    if capacity < 0 || count < 0 || count > capacity || (count != 0 && data == 0) {
        return Err("operation token vector unreadable or invalid");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(TOKEN_STRIDE)
            .ok_or("operation token vector extent overflow")?,
    )
    .ok_or("operation token vector address overflow")?;
    Ok(Some(Entry {
        hash: u32::from_le_bytes(bytes[HASH..HASH + 4].try_into().unwrap()),
        distance: occupied,
        target,
        tokens: Array {
            data,
            count: count as u32,
        },
    }))
}

pub(crate) fn copy_tokens(
    memory: &impl Memory,
    source: Array,
    output: &mut [u32],
) -> Result<(), &'static str> {
    if output.len() != source.count as usize {
        return Err("operation token destination size mismatch");
    }
    let mut bytes = [0; 256];
    for (chunk, destination) in output.chunks_mut(64).enumerate() {
        let address = source
            .data
            .checked_add(
                chunk
                    .checked_mul(bytes.len())
                    .ok_or("operation token vector extent overflow")?,
            )
            .ok_or("operation token vector address overflow")?;
        if !read_slice(
            memory,
            address,
            &mut bytes[..destination.len() * TOKEN_STRIDE],
        ) {
            return Err("operation token vector payload unreadable");
        }
        for (token, value) in destination.iter_mut().zip(bytes.chunks_exact(TOKEN_STRIDE)) {
            *token = u32::from_le_bytes(value.try_into().unwrap());
        }
    }
    Ok(())
}
