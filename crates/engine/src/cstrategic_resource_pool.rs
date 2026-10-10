//! CStrategicResourcePool; slot zero is the resource database's null object.

use crate::cpdx_array::Array;

/// Resource-pool primary vtable.
pub const VTABLE_RVA: usize = 0x293af50;
/// Value-record pointer used by resource-amount predicates.
pub const VALUES: usize = 0x08;
/// Value-record stride selected by resource one-based ordinal.
pub const VALUE_STRIDE: usize = 0x10;
/// Signed i64 amount at the start of each resource record.
pub const VALUE: usize = 0x00;

/// Resource token stored beside the i64 value, copied from its descriptor.
pub const TOKEN: usize = 0x08;

pub(crate) fn from_header(
    base: usize,
    header: [u8; 24],
    count: u32,
) -> Result<Array, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().expect("fixed vtable width")) != expected as u64 {
        return Err("resource pool vtable mismatch");
    }
    let data = usize::try_from(u64::from_le_bytes(
        header[8..16].try_into().expect("fixed pointer width"),
    ))
    .map_err(|_| "resource pool pointer overflow")?;
    let capacity = i32::from_le_bytes(header[16..20].try_into().expect("fixed capacity width"));
    let actual = i32::from_le_bytes(header[20..24].try_into().expect("fixed count width"));
    if actual < 0 || actual > capacity || actual as u32 != count || (count > 0 && data == 0) {
        return Err("resource pool count disagrees with database or capacity");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(VALUE_STRIDE)
            .ok_or("resource pool extent overflow")?,
    )
    .ok_or("resource pool extent overflow")?;
    Ok(Array { data, count })
}

pub(crate) fn from_record(bytes: [u8; 12], token: u32) -> Result<i64, &'static str> {
    let actual_token = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed token width"));
    if actual_token != token {
        return Err("resource value token disagrees with database");
    }
    Ok(i64::from_le_bytes(
        bytes[..8].try_into().expect("fixed value width"),
    ))
}
