//! Air-wing combat records in game memory.

use crate::Memory;

/// Primary record vtable.
pub const VTABLE_RVA: usize = 0x02a9_8f50;
/// Array stride, used for reverse scans and growth.
pub const STRIDE: usize = 0x98;
/// Wing ID, u32.
pub const WING_ID: usize = 0x08;
/// Raw date, i32.
pub const DATE: usize = 0x0c;
/// Plane-count high-water mark, compared and updated as signed i32.
pub const PLANE_COUNT: usize = 0x10;
/// Mission mask, u32.
pub const MISSION_MASK: usize = 0x14;
/// Selection flag byte.
pub const SELECTION_FLAG: usize = 0x18;
/// Classification flag byte.
pub const CLASSIFICATION_FLAG: usize = 0x19;
/// First damage vector, channel ordinal zero.
pub const FIRST_DAMAGE: usize = 0x20;
/// Second damage vector, channel ordinal one.
pub const SECOND_DAMAGE: usize = 0x38;
/// Raw attribution-country link, initialized to zero before nonzero plane-count attribution.
pub const ATTRIBUTED_COUNTRY: usize = 0x50;
/// Copied flag byte.
pub const COPIED_FLAG: usize = 0x90;

/// Full native lookup coordinates, scoped to its physical country/region vector.
/// Lookup matches wing ID and classification among dates >= current hour;
/// the stored date is retained exactly rather than replaced by callback time.
/// Selection is a value, not a lookup discriminator. No pointer or collector
/// generation participates; callers must encode every component without loss.
/// The native loader can append duplicate coordinates. This is not a unique
/// key for arbitrary loaded state; grouped replay entries preserve native order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub country: u32,
    pub region: u32,
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub key: Key,
    /// Native CAirRegionCombatData owner, which may differ from the container.
    pub region_owner: u32,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: u8,
    pub copied: u8,
    /// Zero is the normal initial value before a positive plane high-water mark.
    pub attributed_country: u32,
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    country: u32,
    region: u32,
    side: u8,
    region_owner: u32,
) -> Result<Row, &'static str> {
    let at = |offset| {
        address
            .checked_add(offset)
            .ok_or("air combat record address overflow")
    };
    let mut head = [0; CLASSIFICATION_FLAG + 1];
    let mut attribution = [0; 4];
    let mut copied = [0; 1];
    if memory.copy_scattered(
        [address, at(ATTRIBUTED_COUNTRY)?, at(COPIED_FLAG)?],
        &mut [&mut head, &mut attribution, &mut copied],
    ) != 3
    {
        return Err("air combat record fields unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("air combat record vtable mismatch");
    }
    let word = |offset| u32::from_le_bytes(head[offset..offset + 4].try_into().unwrap());
    let wing_id = word(WING_ID);
    if head[SELECTION_FLAG] > 1 || head[CLASSIFICATION_FLAG] > 1 || copied[0] > 1 {
        return Err("air combat record flag is not boolean");
    }
    Ok(Row {
        key: Key {
            country,
            region,
            side,
            date: word(DATE) as i32,
            wing_id,
            classification: head[CLASSIFICATION_FLAG],
        },
        region_owner,
        planes: word(PLANE_COUNT) as i32,
        mission_mask: word(MISSION_MASK),
        selection: head[SELECTION_FLAG],
        copied: copied[0],
        attributed_country: u32::from_le_bytes(attribution),
    })
}
