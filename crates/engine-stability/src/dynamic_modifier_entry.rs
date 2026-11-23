
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice};
use engine_sites::Rva;

pub const STRIDE: usize = 0x40;
pub const VTABLE_RVA: Rva = Rva(0x0285_3ec8);
pub const SCOPE_COUNTRY: usize = 0x08;
pub const SCOPE_STATE: usize = 0x0c;
pub const DAYS_REMAINING: usize = 0x10;
pub const DEFINITION: usize = 0x18;
pub const ENABLED: usize = 0x20;
pub const VALUES: usize = 0x28;
pub const VALUES_CAPACITY: usize = 0x30;
pub const VALUES_COUNT: usize = 0x34;
pub const VALUE_STRIDE: usize = 8;

pub const CONSTRUCT_RVA: Rva = Rva(0x0059_9670);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub scope_country: u32,
    pub scope_state: i32,
    pub days_remaining: i32,
    pub definition: usize,
    pub enabled: u8,
    pub values: Array,
}

pub fn header(memory: &impl Memory, base: usize, address: usize) -> Result<Header, &'static str> {
    let bytes = read::<[u8; 56]>(memory, address).ok_or("dynamic modifier entry unreadable")?;
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
        return Err("dynamic modifier entry vtable mismatch");
    }
    let bytes = &bytes[8..];
    let scope_country = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let scope_state = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let days_remaining = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let definition = usize::try_from(u64::from_le_bytes(bytes[16..24].try_into().unwrap()))
        .map_err(|_| "dynamic modifier definition pointer overflow")?;
    let enabled = bytes[24];
    let data = usize::try_from(u64::from_le_bytes(bytes[32..40].try_into().unwrap()))
        .map_err(|_| "dynamic modifier value pointer overflow")?;
    let capacity = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[44..48].try_into().unwrap());
    if enabled > 1 || definition == 0 {
        return Err("dynamic modifier enabled byte or definition invalid");
    }
    if count > capacity || capacity > i32::MAX as u32 || (count != 0 && data == 0) {
        return Err("dynamic modifier value array invalid");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(VALUE_STRIDE)
            .ok_or("dynamic modifier value array overflow")?,
    )
    .ok_or("dynamic modifier value array overflow")?;
    Ok(Header {
        scope_country,
        scope_state,
        days_remaining,
        definition,
        enabled,
        values: Array { data, count },
    })
}

pub fn copy_values(memory: &impl Memory, array: Array, output: &mut [i64]) -> bool {
    if output.len() != array.count as usize {
        return false;
    }
    let Some(byte_len) = output.len().checked_mul(VALUE_STRIDE) else {
        return false;
    };
    // SAFETY: initialized i64 has no padding or invalid bit patterns; the byte
    // slice covers exactly the caller-owned output and cannot escape this call.
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), byte_len) };
    if !read_slice(memory, array.data, bytes) {
        return false;
    }
    for value in output {
        *value = i64::from_le(*value);
    }
    true
}
