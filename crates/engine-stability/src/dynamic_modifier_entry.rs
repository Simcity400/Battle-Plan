//! One timed entry in CDynamicModifierContainer's inline array.
//!
//! Remaining duration uses signed daily ticks, separate from absolute expiry.
//! This module describes construction, serialization and vector movement without
//! installing lifecycle hooks.

use crate::{Memory, cpdx_array::Array, read, read_slice};

/// Timed modifier entry stride.
pub const STRIDE: usize = 0x40;
/// Timed modifier entry vtable.
pub const VTABLE_RVA: usize = 0x027b_fd98;
/// Timed modifier scope-country reference u32.
pub const SCOPE_COUNTRY: usize = 0x08;
/// Timed modifier scope-state i32.
pub const SCOPE_STATE: usize = 0x0c;
/// Timed modifier remaining daily ticks i32.
pub const DAYS_REMAINING: usize = 0x10;
/// Permanent-duration sentinel; distinct from zero remaining ticks.
pub const PERMANENT_DAYS: i32 = -1;
/// Timed modifier CDynamicModifier definition pointer.
pub const DEFINITION: usize = 0x18;
/// Enabled byte; accepts only zero or one.
pub const ENABLED: usize = 0x20;
/// Timed modifier i64 value-array data pointer.
pub const VALUES: usize = 0x28;
/// Timed modifier value-array capacity u32.
pub const VALUES_CAPACITY: usize = 0x30;
/// Timed modifier value-array count u32.
pub const VALUES_COUNT: usize = 0x34;
/// Timed modifier value-array element stride.
pub const VALUE_STRIDE: usize = 8;
/// Timed modifier signed fixed-point divisor.
pub const VALUE_SCALE: i64 = 100_000;

/// Timed entry construction/layout.
pub const CONSTRUCT_RVA: usize = 0x0054_5ca0;
/// Timed entry vector erase/move.
pub const ERASE_RANGE_RVA: usize = 0x0060_4d30;
/// Vector-movement entry bytes.
pub const ERASE_RANGE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b,
];
/// Timed entry serializer.
pub const SERIALIZE_RVA: usize = 0x0060_5c80;
/// Timed entry deserializer field dispatcher.
pub const DESERIALIZE_RVA: usize = 0x0060_3db0;
/// Timed entry definition-sized value-vector materializer.
pub const MATERIALIZE_VALUES_RVA: usize = 0x005f_fd00;
/// Timed entry i64 value-vector deserializer.
pub const DESERIALIZE_VALUES_RVA: usize = 0x0060_4600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub scope_country: u32,
    pub scope_state: i32,
    pub days_remaining: i32,
    pub definition: usize,
    pub enabled: u8,
    pub values: Array,
}

/// Reads the complete timed-instance field range once. The caller owns the
/// hourly consistency protocol; this reader does not retry a moving entry.
pub fn header(memory: &impl Memory, base: usize, address: usize) -> Result<Header, &'static str> {
    let bytes = read::<[u8; 56]>(memory, address).ok_or("dynamic modifier entry unreadable")?;
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
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
