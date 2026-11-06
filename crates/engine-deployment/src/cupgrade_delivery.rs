//! CUpgradeDelivery with an embedded CEquipmentVariantPool.
//!
//! Progress is serialized only for status one. The ordinary hourly path
//! updates and removes deliveries before capture; other allocation and command
//! routes do not establish a consistency contract here.

use engine_sites::Rva;
/// CUpgradeDelivery primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x2a4b900);
/// CUpgradeDelivery status, u32.
pub const STATUS: usize = 0x08;
/// Status whose progress is read; other statuses do not reach the progress field.
pub const PROGRESS_AVAILABLE_STATUS: u32 = 1;
/// CUpgradeDelivery progress, i64 native fixed point.
pub const PROGRESS: usize = 0x10;
/// CUpgradeDelivery total progress, i64 native fixed point.
pub const TOTAL_PROGRESS: usize = 0x18;
/// CUpgradeDelivery embedded CEquipmentVariantPool.
pub const PRODUCED_EQUIPMENT_POOL: usize = 0x20;
/// CUpgradeDelivery numeric scale.
pub const FIXED_POINT_SCALE: i64 = 100_000;

use crate::{
    Memory, cequipment_variant, cequipment_variant_pool, cpdx_array::Array, equipment_entry, read,
};

/// The engine serializes deliveries in vector order; no persistent delivery
/// id has been established. A non-progress status retains None, never zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub status: u32,
    pub progress_raw: Option<i64>,
    pub total_progress_raw: i64,
    pub allow_zero_entries: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProducedRow {
    pub country: u32,
    pub army: u64,
    pub delivery_ordinal: u32,
    pub ordinal: u32,
    pub equipment: u64,
    pub amount_raw: i64,
}

pub(crate) struct Header {
    address: usize,
    status: Option<u32>,
    pool: usize,
}

pub(crate) fn authenticate(
    memory: &impl Memory,
    base: usize,
    address: usize,
) -> Result<Header, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let expected_pool = engine_sites::at(base, cequipment_variant_pool::VTABLE_RVA)
        .ok_or("module address overflow")?;
    let pool = address
        .checked_add(PRODUCED_EQUIPMENT_POOL)
        .ok_or("army upgrade produced pool address overflow")?;
    let mut identity = [0_u8; 8];
    let mut pool_vtable = [0_u8; 8];
    let mut status = [0_u8; 4];
    let status_at = address
        .checked_add(STATUS)
        .ok_or("army upgrade delivery address overflow")?;
    let copied = memory.copy_scattered(
        [address, pool, status_at],
        &mut [&mut identity, &mut pool_vtable, &mut status],
    );
    if copied < 2 {
        return Err("army upgrade delivery identity unreadable");
    }
    if u64::from_le_bytes(identity) != expected as u64 {
        return Err("army upgrade delivery vtable mismatch");
    }
    if u64::from_le_bytes(pool_vtable) != expected_pool as u64 {
        return Err("army upgrade produced pool vtable mismatch");
    }
    Ok(Header {
        address,
        pool,
        // A status-field fault does not invalidate the independently
        // authenticated produced pool. Its own table can still be read.
        status: (copied == 3).then_some(u32::from_le_bytes(status)),
    })
}

pub(crate) fn row(
    memory: &impl Memory,
    header: &Header,
    country: u32,
    army: u64,
    ordinal: u32,
) -> Result<Row, &'static str> {
    let status = header
        .status
        .ok_or("army upgrade delivery status unreadable")?;
    let at = |offset| {
        header
            .address
            .checked_add(offset)
            .ok_or("army upgrade delivery address overflow")
    };
    let mut progress = [0_u8; 8];
    let mut total = [0_u8; 8];
    let mut allow_zero = [0_u8; 1];
    let progress_len = if status == PROGRESS_AVAILABLE_STATUS {
        8
    } else {
        0
    };
    let allow_zero_at = header
        .pool
        .checked_add(cequipment_variant_pool::ALLOW_ZERO)
        .ok_or("army upgrade produced pool address overflow")?;
    // An empty first span never touches the inactive progress field. The
    // scatter result preserves which exact field failed without a retry.
    match memory.copy_scattered(
        [at(PROGRESS)?, at(TOTAL_PROGRESS)?, allow_zero_at],
        &mut [&mut progress[..progress_len], &mut total, &mut allow_zero],
    ) {
        0 => return Err("army upgrade delivery progress unreadable"),
        1 => return Err("army upgrade delivery total progress unreadable"),
        2 => return Err("army upgrade produced allow-zero flag unreadable"),
        _ => {}
    }
    let progress_raw =
        (status == PROGRESS_AVAILABLE_STATUS).then_some(i64::from_le_bytes(progress));
    let total_progress_raw = i64::from_le_bytes(total);
    let allow_zero_entries = allow_zero[0];
    Ok(Row {
        country,
        army,
        ordinal,
        status,
        progress_raw,
        total_progress_raw,
        allow_zero_entries,
    })
}

pub(crate) fn produced(memory: &impl Memory, header: &Header) -> Result<Array, &'static str> {
    header
        .pool
        .checked_add(cequipment_variant_pool::ITEMS)
        .and_then(|p| Array::read(memory, p, equipment_entry::STRIDE))
        .ok_or("army upgrade produced vector unreadable or invalid")
}

pub(crate) fn produced_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    parent: (u32, u64, u32),
    ordinal: u32,
) -> Result<ProducedRow, &'static str> {
    let bytes = read::<[u8; equipment_entry::STRIDE]>(memory, address)
        .ok_or("army upgrade produced entry unreadable")?;
    let pointer = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .map_err(|_| "army upgrade produced equipment pointer overflow")?;
    Ok(ProducedRow {
        country: parent.0,
        army: parent.1,
        delivery_ordinal: parent.2,
        ordinal,
        equipment: cequipment_variant::identity(memory, base, pointer)?,
        amount_raw: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
    })
}
