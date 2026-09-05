use std::collections::BTreeSet;
use compact_contract::owner_colour::{self, OwnerColourRow, OwnerColourStatus};

use crate::{BattleplansDepthMemory, ARMY_VTABLE_RVA};

pub const REFERENCE_TABLE_ARRAY_RVA: u64 = 0x3438_E60;
pub const ORDERS_GROUP_REFERENCE_TYPE: u32 = 53;
pub const ORDERS_GROUP_TABLE_PTR_RVA: u64 =
    REFERENCE_TABLE_ARRAY_RVA + ORDERS_GROUP_REFERENCE_TYPE as u64 * 8;

pub const ARMY_GROUP_VTABLE_RVA: u64 = 0x293_BCF8;
pub const ORDERS_GROUP_VTABLE_RVA: u64 = ARMY_VTABLE_RVA;
pub const ORDERS_GROUP_OFF_HANDLE: u64 = 0x08;
pub const ORDERS_GROUP_OFF_EXECUTION_TYPE: u64 = 0x1A4;
pub const ORDERS_GROUP_OFF_COHESION_TYPE: u64 = 0x1A8;
pub const ORDERS_GROUP_OFF_LEADER_PROXIMITY: u64 = 0x1AC;

const TABLE_OFF_ENTRIES: u64 = 0x08;
const TABLE_OFF_COUNT: u64 = 0x10;
const TABLE_OFF_MASK: u64 = 0x14;
const TABLE_OFF_MAX_DISTANCE: u64 = 0x18;
const ENTRY_STRIDE: usize = 0x18;
const ENTRY_OFF_DISTANCE_PLUS_ONE: usize = 0x04;
const ENTRY_OFF_HANDLE: usize = 0x08;
const ENTRY_OFF_OBJECT: usize = 0x10;
const ITERATOR_SENTINEL_DISTANCE: u8 = 0xFF;
const MAX_TABLE_CAPACITY: u32 = 262_144;
const MAX_ARMY_ID: u32 = i32::MAX as u32;

pub const EXTENSION_MAGIC: [u8; 4] = *b"OGS1";
pub const EXTENSION_VERSION: u16 = owner_colour::OGS_VERSION;
pub const EXTENSION_HEADER_LEN: usize = 24;
pub const EXTENSION_ROW_LEN: usize = 16;
pub const EXTENSION_FLAG_COMPLETE_MANAGER: u32 = 1;
pub const ORDER_OWNER_KIND_ARMY: u32 = 0;
pub const ORDER_OWNER_KIND_ARMY_GROUP: u32 = 1;

pub const MAX_TYPED_EXECUTION_TYPE: u32 = 2;
pub const MAX_COHESION_TYPE: u32 = 3;
pub const MAX_LEADER_PROXIMITY: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupRow {
    pub army_id: u32,
    pub execution_type: u32,
    pub cohesion_type: u32,
    pub leader_proximity: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompleteArmyOrdersGroups {
    pub owner_colours: Vec<OwnerColourRow>,
    pub armies: Vec<ArmyOrdersGroupRow>,
    pub army_group_count: u32,
    pub army_group_ids: Vec<u32>,
    pub registered_entry_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrdersGroupRoot {
    pub owner_kind: u32,
    pub owner_id: u32,
    pub object: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompleteArmyOrdersGroupRoots {
    pub roots: Vec<ArmyOrdersGroupRoot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrdersGroupReadError {
    AddressOverflow,
    Unreadable(&'static str),
    UnstableTable,
    InvalidTable(&'static str),
    InvalidEntry { index: u32, reason: &'static str },
    InvalidObject { army_id: u32, reason: &'static str },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodedOrdersGroupExtension {
    pub owner_colours: Vec<OwnerColourRow>,
    pub complete_manager: bool,
    pub owner_identities_complete: bool,
    pub army_group_count: u32,
    pub army_group_ids: Vec<u32>,
    pub registered_entry_count: u32,
    pub armies: Vec<ArmyOrdersGroupRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableHeader {
    table: u64,
    entries: u64,
    count: u32,
    mask: u32,
    max_distance: u8,
}

pub fn read_complete_army_orders_groups<M: BattleplansDepthMemory>(
    mem: &M,
) -> Result<CompleteArmyOrdersGroups, OrdersGroupReadError> {
    let read = read_owner_census(mem, true)?;
    Ok(CompleteArmyOrdersGroups {
        owner_colours: read.owner_colours, armies: read.armies,
        army_group_count: read.army_group_count, army_group_ids: read.army_group_ids,
        registered_entry_count: read.registered_entry_count,
    })
}

pub fn read_complete_army_orders_group_roots<M: BattleplansDepthMemory>(
    mem: &M,
) -> Result<CompleteArmyOrdersGroupRoots, OrdersGroupReadError> {
    read_owner_census(mem, false).map(|read| CompleteArmyOrdersGroupRoots { roots: read.roots })
}

struct OwnerCensusRead {
    owner_colours: Vec<OwnerColourRow>,
    armies: Vec<ArmyOrdersGroupRow>,
    army_group_count: u32,
    army_group_ids: Vec<u32>,
    registered_entry_count: u32,
    roots: Vec<ArmyOrdersGroupRoot>,
}

fn read_owner_census<M: BattleplansDepthMemory>(
    mem: &M, capture_colour: bool,
) -> Result<OwnerCensusRead, OrdersGroupReadError> {
    let table_ptr_address = mem
        .module_base()
        .checked_add(ORDERS_GROUP_TABLE_PTR_RVA)
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    let header_before = read_table_header(mem, table_ptr_address)?;
    let terminal_index = validated_terminal_index(&header_before)?;
    let byte_len = usize::try_from(terminal_index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .and_then(|slots| slots.checked_mul(ENTRY_STRIDE))
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    let snapshot_a = mem
        .read_bytes(header_before.entries, byte_len)
        .ok_or(OrdersGroupReadError::Unreadable("reference table entries"))?;
    if snapshot_a.len() != byte_len {
        return Err(OrdersGroupReadError::Unreadable("reference table entries"));
    }
    let header_middle = read_table_header(mem, table_ptr_address)?;
    if header_before != header_middle {
        return Err(OrdersGroupReadError::UnstableTable);
    }
    let snapshot_b = mem
        .read_bytes(header_before.entries, byte_len)
        .ok_or(OrdersGroupReadError::Unreadable("reference table entries"))?;
    let header_after = read_table_header(mem, table_ptr_address)?;
    if header_middle != header_after || snapshot_a != snapshot_b {
        return Err(OrdersGroupReadError::UnstableTable);
    }

    let sentinel_offset = terminal_index as usize * ENTRY_STRIDE;
    if snapshot_a[sentinel_offset + ENTRY_OFF_DISTANCE_PLUS_ONE] != ITERATOR_SENTINEL_DISTANCE {
        return Err(OrdersGroupReadError::InvalidTable(
            "missing terminal iterator sentinel",
        ));
    }

    let orders_group_vtable = mem
        .module_base()
        .checked_add(ORDERS_GROUP_VTABLE_RVA)
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    let army_group_vtable = mem
        .module_base()
        .checked_add(ARMY_GROUP_VTABLE_RVA)
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    let mut armies = Vec::new();
    let mut roots = Vec::new();
    let mut owner_colours = Vec::new();
    let mut army_group_ids = Vec::new();
    let mut registered_ids = BTreeSet::new();
    let mut army_group_count = 0u32;
    let mut occupied_count = 0u32;

    for index in 0..terminal_index as usize {
        let entry = &snapshot_a[index * ENTRY_STRIDE..(index + 1) * ENTRY_STRIDE];
        let distance = entry[ENTRY_OFF_DISTANCE_PLUS_ONE];
        if distance == 0 {
            continue;
        }
        if distance == ITERATOR_SENTINEL_DISTANCE {
            return Err(OrdersGroupReadError::InvalidEntry {
                index: index as u32,
                reason: "iterator sentinel before terminal entry",
            });
        }
        if u16::from(distance) > u16::from(header_before.max_distance) + 1 {
            return Err(OrdersGroupReadError::InvalidEntry {
                index: index as u32,
                reason: "probe distance exceeds table bound",
            });
        }
        occupied_count = occupied_count
            .checked_add(1)
            .ok_or(OrdersGroupReadError::AddressOverflow)?;
        let handle = slice_u64(entry, ENTRY_OFF_HANDLE);
        let kind = handle as u32;
        let id = (handle >> 32) as u32;
        if kind != ORDERS_GROUP_REFERENCE_TYPE || id == 0 || id > MAX_ARMY_ID {
            return Err(OrdersGroupReadError::InvalidEntry {
                index: index as u32,
                reason: "invalid type-53 handle",
            });
        }
        if !registered_ids.insert(id) {
            return Err(OrdersGroupReadError::InvalidEntry {
                index: index as u32,
                reason: "duplicate registered handle id",
            });
        }
        let object = slice_u64(entry, ENTRY_OFF_OBJECT);
        if !mem.is_heap_pointer(object) {
            return Err(OrdersGroupReadError::InvalidEntry {
                index: index as u32,
                reason: "object is not a heap pointer",
            });
        }

        let identity_a = mem
            .read_bytes(object, 16)
            .ok_or(OrdersGroupReadError::InvalidObject {
                army_id: id,
                reason: "unreadable identity",
            })?;
        if identity_a.len() != 16 || slice_u64(&identity_a, 8) != handle {
            return Err(OrdersGroupReadError::InvalidObject {
                army_id: id,
                reason: "entry/object handle mismatch",
            });
        }
        let vtable = slice_u64(&identity_a, 0);
        if capture_colour && (vtable == army_group_vtable || vtable == orders_group_vtable) {
            owner_colours.push(read_owner_colour(mem, object, &identity_a,
                u32::from(vtable == army_group_vtable), id));
        }
        if vtable == army_group_vtable {
            let identity_b =
                mem.read_bytes(object, 16)
                    .ok_or(OrdersGroupReadError::InvalidObject {
                        army_id: id,
                        reason: "unreadable identity",
                    })?;
            if identity_a != identity_b {
                return Err(OrdersGroupReadError::InvalidObject {
                    army_id: id,
                    reason: "object changed during read",
                });
            }
            army_group_count = army_group_count
                .checked_add(1)
                .ok_or(OrdersGroupReadError::AddressOverflow)?;
            army_group_ids.push(id);
            roots.push(ArmyOrdersGroupRoot {
                owner_kind: ORDER_OWNER_KIND_ARMY_GROUP,
                owner_id: id,
                object,
            });
        } else if vtable == orders_group_vtable {
            let scalars_address = object
                .checked_add(ORDERS_GROUP_OFF_EXECUTION_TYPE)
                .ok_or(OrdersGroupReadError::AddressOverflow)?;
            let scalars_a =
                mem.read_bytes(scalars_address, 12)
                    .ok_or(OrdersGroupReadError::InvalidObject {
                        army_id: id,
                        reason: "unreadable order-group scalars",
                    })?;
            let scalars_b =
                mem.read_bytes(scalars_address, 12)
                    .ok_or(OrdersGroupReadError::InvalidObject {
                        army_id: id,
                        reason: "unreadable order-group scalars",
                    })?;
            let identity_b =
                mem.read_bytes(object, 16)
                    .ok_or(OrdersGroupReadError::InvalidObject {
                        army_id: id,
                        reason: "unreadable identity",
                    })?;
            if scalars_a.len() != 12 || scalars_a != scalars_b || identity_a != identity_b {
                return Err(OrdersGroupReadError::InvalidObject {
                    army_id: id,
                    reason: "object changed during read",
                });
            }
            let execution_type = slice_u32(&scalars_a, 0);
            let cohesion_type = slice_u32(&scalars_a, 4);
            let leader_proximity = slice_u32(&scalars_a, 8);
            if cohesion_type > MAX_COHESION_TYPE {
                return Err(OrdersGroupReadError::InvalidObject {
                    army_id: id,
                    reason: "cohesion type outside exact UI enum",
                });
            }
            if leader_proximity > MAX_LEADER_PROXIMITY {
                return Err(OrdersGroupReadError::InvalidObject {
                    army_id: id,
                    reason: "leader proximity outside exact UI enum",
                });
            }
            armies.push(ArmyOrdersGroupRow {
                army_id: id,
                execution_type,
                cohesion_type,
                leader_proximity,
            });
            roots.push(ArmyOrdersGroupRoot {
                owner_kind: ORDER_OWNER_KIND_ARMY,
                owner_id: id,
                object,
            });
        } else {
            return Err(OrdersGroupReadError::InvalidObject {
                army_id: id,
                reason: "unknown type-53 vtable",
            });
        }
    }

    let header_final_before = read_table_header(mem, table_ptr_address)?;
    if header_final_before != header_before {
        return Err(OrdersGroupReadError::UnstableTable);
    }
    let snapshot_final = mem
        .read_bytes(header_before.entries, byte_len)
        .ok_or(OrdersGroupReadError::Unreadable("reference table entries"))?;
    let header_final_after = read_table_header(mem, table_ptr_address)?;
    if header_final_after != header_before || snapshot_final != snapshot_a {
        return Err(OrdersGroupReadError::UnstableTable);
    }

    if occupied_count != header_before.count {
        return Err(OrdersGroupReadError::InvalidTable(
            "occupied entry count does not match header",
        ));
    }
    armies.sort_by_key(|row| row.army_id);
    army_group_ids.sort_unstable();
    roots.sort_by_key(|row| (row.owner_kind, row.owner_id));
    owner_colours.sort_by_key(|row| (row.owner_kind, row.owner_id));
    Ok(OwnerCensusRead {
        owner_colours, armies, army_group_count, army_group_ids,
        registered_entry_count: occupied_count, roots,
    })
}

fn read_owner_colour<M: BattleplansDepthMemory>(mem: &M, object: u64, identity: &[u8], owner_kind: u32, owner_id: u32) -> OwnerColourRow {
    let mut row = OwnerColourRow { owner_kind, owner_id, status: OwnerColourStatus::Unreadable, rgba_f32_bits: [0; 4] };
    let Some(address) = object.checked_add(0x110) else { return row; };
    let (Some(first), Some(second), Some(after)) =
        (mem.read_bytes(address, 16), mem.read_bytes(address, 16), mem.read_bytes(object, 16)) else { return row; };
    if first.len() != 16 || second.len() != 16 || after.len() != 16 { return row; }
    if first != second || after != identity {
        row.status = OwnerColourStatus::Unstable;
        return row;
    }
    row.rgba_f32_bits = std::array::from_fn(|axis| slice_u32(&first, axis * 4));
    row.status = if row.rgba_f32_bits.iter().all(|bits| f32::from_bits(*bits).is_finite()) {
        OwnerColourStatus::Exact
    } else { OwnerColourStatus::NonFinite };
    row
}

pub fn append_orders_group_extension(
    payload: &mut Vec<u8>,
    carrier: &CompleteArmyOrdersGroups,
) -> Result<(), OrdersGroupReadError> {
    validate_carrier(carrier)?;
    payload.extend_from_slice(&EXTENSION_MAGIC);
    payload.extend_from_slice(&EXTENSION_VERSION.to_le_bytes());
    payload.extend_from_slice(&(EXTENSION_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(carrier.armies.len() as u32).to_le_bytes());
    payload.extend_from_slice(&carrier.registered_entry_count.to_le_bytes());
    payload.extend_from_slice(&carrier.army_group_count.to_le_bytes());
    payload.extend_from_slice(&EXTENSION_FLAG_COMPLETE_MANAGER.to_le_bytes());
    for row in &carrier.armies {
        payload.extend_from_slice(&row.army_id.to_le_bytes());
        payload.extend_from_slice(&row.execution_type.to_le_bytes());
        payload.extend_from_slice(&row.cohesion_type.to_le_bytes());
        payload.extend_from_slice(&row.leader_proximity.to_le_bytes());
    }
    for army_group_id in &carrier.army_group_ids {
        payload.extend_from_slice(&army_group_id.to_le_bytes());
    }
    owner_colour::encode(&carrier.owner_colours, payload);
    Ok(())
}

pub fn decode_orders_group_extension(
    payload: &[u8],
) -> Result<DecodedOrdersGroupExtension, &'static str> {
    if payload.len() < EXTENSION_HEADER_LEN {
        return Err("orders-group extension shorter than header");
    }
    if payload[..4] != EXTENSION_MAGIC {
        return Err("orders-group extension has bad magic");
    }
    let version = slice_u16(payload, 4);
    if version != EXTENSION_VERSION {
        return Err("orders-group extension has unsupported version");
    }
    if slice_u16(payload, 6) as usize != EXTENSION_HEADER_LEN {
        return Err("orders-group extension has bad header length");
    }
    let row_count = slice_u32(payload, 8) as usize;
    let registered_entry_count = slice_u32(payload, 12);
    let army_group_count = slice_u32(payload, 16);
    let flags = slice_u32(payload, 20);
    if flags != EXTENSION_FLAG_COMPLETE_MANAGER {
        return Err("orders-group extension is not complete-manager authority");
    }
    let army_rows_end = row_count
        .checked_mul(EXTENSION_ROW_LEN)
        .and_then(|rows| EXTENSION_HEADER_LEN.checked_add(rows))
        .ok_or("orders-group extension length overflow")?;
    let owner_colours_start = {
        (army_group_count as usize)
            .checked_mul(4)
            .and_then(|bytes| army_rows_end.checked_add(bytes))
            .ok_or("orders-group extension length overflow")?
    };
    let expected = (registered_entry_count as usize).checked_mul(owner_colour::ROW_BYTES)
        .and_then(|bytes| owner_colours_start.checked_add(bytes)).ok_or("orders-group colour length overflow")?;
    if payload.len() != expected {
        return Err("orders-group extension length mismatch");
    }
    if registered_entry_count != (row_count as u32).saturating_add(army_group_count) {
        return Err("orders-group extension registered count mismatch");
    }
    if registered_entry_count as usize > owner_colour::MAX_OWNERS {
        return Err("orders-group owner census exceeds bound");
    }
    let mut armies = Vec::with_capacity(row_count);
    let mut previous_id = 0u32;
    for index in 0..row_count {
        let base = EXTENSION_HEADER_LEN + index * EXTENSION_ROW_LEN;
        let army_id = slice_u32(payload, base);
        let execution_type = slice_u32(payload, base + 4);
        let cohesion_type = slice_u32(payload, base + 8);
        let leader_proximity = slice_u32(payload, base + 12);
        if army_id == 0 || army_id > MAX_ARMY_ID || army_id <= previous_id {
            return Err("orders-group extension army ids are not strict sorted ids");
        }
        if cohesion_type > MAX_COHESION_TYPE || leader_proximity > MAX_LEADER_PROXIMITY {
            return Err("orders-group extension scalar outside exact UI enum");
        }
        previous_id = army_id;
        armies.push(ArmyOrdersGroupRow {
            army_id,
            execution_type,
            cohesion_type,
            leader_proximity,
        });
    }
    let army_ids = armies
        .iter()
        .map(|row| row.army_id)
        .collect::<BTreeSet<_>>();
    let mut army_group_ids = Vec::with_capacity(army_group_count as usize);
    let mut previous_group_id = 0u32;
    for index in 0..army_group_count as usize {
        let army_group_id = slice_u32(payload, army_rows_end + index * 4);
        if army_group_id == 0
            || army_group_id > MAX_ARMY_ID
            || army_group_id <= previous_group_id
            || army_ids.contains(&army_group_id)
        {
            return Err("orders-group extension army-group ids are not strict disjoint ids");
        }
        previous_group_id = army_group_id;
        army_group_ids.push(army_group_id);
    }
    let owner_colours = owner_colour::decode(&payload[owner_colours_start..],
        armies.iter().map(|row| (0, row.army_id)).chain(army_group_ids.iter().map(|id| (1, *id))))?;
    Ok(DecodedOrdersGroupExtension {
        owner_colours,
        complete_manager: true,
        owner_identities_complete: true,
        army_group_count,
        army_group_ids,
        registered_entry_count,
        armies,
    })
}

fn validate_carrier(carrier: &CompleteArmyOrdersGroups) -> Result<(), OrdersGroupReadError> {
    owner_colour::validate(&carrier.owner_colours, carrier.armies.iter().map(|row| (0, row.army_id))
        .chain(carrier.army_group_ids.iter().map(|id| (1, *id))))
        .map_err(OrdersGroupReadError::InvalidTable)?;
    let army_count = u32::try_from(carrier.armies.len())
        .map_err(|_| OrdersGroupReadError::InvalidTable("army row count overflow"))?;
    if army_count.saturating_add(carrier.army_group_count) != carrier.registered_entry_count {
        return Err(OrdersGroupReadError::InvalidTable(
            "registered count does not equal army plus army-group rows",
        ));
    }
    if carrier.army_group_ids.len() != carrier.army_group_count as usize {
        return Err(OrdersGroupReadError::InvalidTable(
            "army-group id count does not match army-group rows",
        ));
    }
    let mut previous_id = 0u32;
    for row in &carrier.armies {
        if row.army_id == 0 || row.army_id > MAX_ARMY_ID || row.army_id <= previous_id {
            return Err(OrdersGroupReadError::InvalidObject {
                army_id: row.army_id,
                reason: "army ids are not strict sorted ids",
            });
        }
        if row.cohesion_type > MAX_COHESION_TYPE || row.leader_proximity > MAX_LEADER_PROXIMITY {
            return Err(OrdersGroupReadError::InvalidObject {
                army_id: row.army_id,
                reason: "scalar outside exact UI enum",
            });
        }
        previous_id = row.army_id;
    }
    let army_ids = carrier
        .armies
        .iter()
        .map(|row| row.army_id)
        .collect::<BTreeSet<_>>();
    let mut previous_group_id = 0u32;
    for army_group_id in &carrier.army_group_ids {
        if *army_group_id == 0
            || *army_group_id > MAX_ARMY_ID
            || *army_group_id <= previous_group_id
            || army_ids.contains(army_group_id)
        {
            return Err(OrdersGroupReadError::InvalidObject {
                army_id: *army_group_id,
                reason: "army-group ids are not strict disjoint ids",
            });
        }
        previous_group_id = *army_group_id;
    }
    Ok(())
}

fn read_table_header<M: BattleplansDepthMemory>(
    mem: &M,
    table_ptr_address: u64,
) -> Result<TableHeader, OrdersGroupReadError> {
    let table = mem
        .read_u64(table_ptr_address)
        .ok_or(OrdersGroupReadError::Unreadable("type-53 table pointer"))?;
    if !mem.is_heap_pointer(table) {
        return Err(OrdersGroupReadError::InvalidTable(
            "type-53 table is not a heap pointer",
        ));
    }
    let entries = mem
        .read_u64(
            table
                .checked_add(TABLE_OFF_ENTRIES)
                .ok_or(OrdersGroupReadError::AddressOverflow)?,
        )
        .ok_or(OrdersGroupReadError::Unreadable("table entries pointer"))?;
    if !mem.is_heap_pointer(entries) {
        return Err(OrdersGroupReadError::InvalidTable(
            "entries is not a heap pointer",
        ));
    }
    let count = mem
        .read_u32(
            table
                .checked_add(TABLE_OFF_COUNT)
                .ok_or(OrdersGroupReadError::AddressOverflow)?,
        )
        .ok_or(OrdersGroupReadError::Unreadable("table count"))?;
    let mask = mem
        .read_u32(
            table
                .checked_add(TABLE_OFF_MASK)
                .ok_or(OrdersGroupReadError::AddressOverflow)?,
        )
        .ok_or(OrdersGroupReadError::Unreadable("table mask"))?;
    let max_distance = read_u8(mem, table, TABLE_OFF_MAX_DISTANCE, "table max distance")?;
    Ok(TableHeader {
        table,
        entries,
        count,
        mask,
        max_distance,
    })
}

fn validated_terminal_index(header: &TableHeader) -> Result<u32, OrdersGroupReadError> {
    let capacity = header
        .mask
        .checked_add(1)
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    if !capacity.is_power_of_two() || capacity > MAX_TABLE_CAPACITY {
        return Err(OrdersGroupReadError::InvalidTable(
            "mask is not a bounded power-of-two capacity",
        ));
    }
    if header.count > capacity {
        return Err(OrdersGroupReadError::InvalidTable(
            "entry count exceeds table capacity",
        ));
    }
    header
        .mask
        .checked_add(header.max_distance as u32)
        .and_then(|value| value.checked_add(1))
        .ok_or(OrdersGroupReadError::AddressOverflow)
}

fn read_u8<M: BattleplansDepthMemory>(
    mem: &M,
    root: u64,
    offset: u64,
    field: &'static str,
) -> Result<u8, OrdersGroupReadError> {
    let address = root
        .checked_add(offset)
        .ok_or(OrdersGroupReadError::AddressOverflow)?;
    let bytes = mem
        .read_bytes(address, 1)
        .ok_or(OrdersGroupReadError::Unreadable(field))?;
    bytes
        .first()
        .copied()
        .ok_or(OrdersGroupReadError::Unreadable(field))
}

fn slice_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn slice_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn slice_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const MODULE: u64 = 0x1400_0000_0;
    const TABLE: u64 = 0x2000_0000_0;
    const ENTRIES: u64 = 0x2000_0100_0;
    const ARMY: u64 = 0x2000_0200_0;
    const ARMY_GROUP: u64 = 0x2000_0300_0;

    #[derive(Default)]
    struct TestMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl TestMemory {
        fn put(&mut self, address: u64, bytes: &[u8]) {
            for (index, value) in bytes.iter().copied().enumerate() {
                self.bytes.insert(address + index as u64, value);
            }
        }

        fn put_u32(&mut self, address: u64, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        fn put_u64(&mut self, address: u64, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
    }

    impl BattleplansDepthMemory for TestMemory {
        fn module_base(&self) -> u64 {
            MODULE
        }

        fn read_u64(&self, address: u64) -> Option<u64> {
            self.read_bytes(address, 8)
                .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
        }

        fn read_u32(&self, address: u64) -> Option<u32> {
            self.read_bytes(address, 4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        }

        fn read_i32(&self, address: u64) -> Option<i32> {
            self.read_bytes(address, 4)
                .map(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()))
        }

        fn read_bytes(&self, address: u64, len: usize) -> Option<Vec<u8>> {
            (0..len)
                .map(|index| self.bytes.get(&(address + index as u64)).copied())
                .collect()
        }

        fn is_heap_pointer(&self, address: u64) -> bool {
            (0x2000_0000_0..0x3000_0000_0).contains(&address)
        }
    }

    fn handle(id: u32) -> u64 {
        ((id as u64) << 32) | ORDERS_GROUP_REFERENCE_TYPE as u64
    }

    fn exact_fixture() -> TestMemory {
        let mut mem = TestMemory::default();
        mem.put_u64(MODULE + ORDERS_GROUP_TABLE_PTR_RVA, TABLE);
        mem.put_u64(TABLE + TABLE_OFF_ENTRIES, ENTRIES);
        mem.put_u32(TABLE + TABLE_OFF_COUNT, 2);
        mem.put_u32(TABLE + TABLE_OFF_MASK, 3);
        mem.put(TABLE + TABLE_OFF_MAX_DISTANCE, &[0]);
        mem.put(ENTRIES, &[0; 5 * ENTRY_STRIDE]);

        let army_entry = ENTRIES;
        mem.put(army_entry + ENTRY_OFF_DISTANCE_PLUS_ONE as u64, &[1]);
        mem.put_u64(army_entry + ENTRY_OFF_HANDLE as u64, handle(17));
        mem.put_u64(army_entry + ENTRY_OFF_OBJECT as u64, ARMY);
        let group_entry = ENTRIES + 2 * ENTRY_STRIDE as u64;
        mem.put(group_entry + ENTRY_OFF_DISTANCE_PLUS_ONE as u64, &[1]);
        mem.put_u64(group_entry + ENTRY_OFF_HANDLE as u64, handle(23));
        mem.put_u64(group_entry + ENTRY_OFF_OBJECT as u64, ARMY_GROUP);
        let sentinel = ENTRIES + 4 * ENTRY_STRIDE as u64;
        mem.put(
            sentinel + ENTRY_OFF_DISTANCE_PLUS_ONE as u64,
            &[ITERATOR_SENTINEL_DISTANCE],
        );

        mem.put_u64(ARMY, MODULE + ORDERS_GROUP_VTABLE_RVA);
        mem.put_u64(ARMY + ORDERS_GROUP_OFF_HANDLE, handle(17));
        mem.put_u32(ARMY + ORDERS_GROUP_OFF_EXECUTION_TYPE, 2);
        mem.put_u32(ARMY + ORDERS_GROUP_OFF_COHESION_TYPE, 3);
        mem.put_u32(ARMY + ORDERS_GROUP_OFF_LEADER_PROXIMITY, 4);
        mem.put_u64(ARMY_GROUP, MODULE + ARMY_GROUP_VTABLE_RVA);
        mem.put_u64(ARMY_GROUP + ORDERS_GROUP_OFF_HANDLE, handle(23));
        mem
    }

    #[test]
    fn enumerates_armies_without_requiring_any_order_rows() {
        let rows = read_complete_army_orders_groups(&exact_fixture()).unwrap();
        assert_eq!(rows.registered_entry_count, 2);
        assert_eq!(rows.army_group_count, 1);
        assert_eq!(rows.army_group_ids, vec![23]);
        assert_eq!(
            rows.armies,
            vec![ArmyOrdersGroupRow {
                army_id: 17,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            }]
        );
    }

    #[test]
    fn complete_census_exposes_typed_process_roots_for_both_type_53_vtables() {
        let read = read_complete_army_orders_group_roots(&exact_fixture()).unwrap();
        assert_eq!(read.roots.len(), 2);
        assert_eq!(
            read.roots,
            vec![
                ArmyOrdersGroupRoot {
                    owner_kind: ORDER_OWNER_KIND_ARMY,
                    owner_id: 17,
                    object: ARMY,
                },
                ArmyOrdersGroupRoot {
                    owner_kind: ORDER_OWNER_KIND_ARMY_GROUP,
                    owner_id: 23,
                    object: ARMY_GROUP,
                },
            ]
        );
    }

    #[test]
    fn preserves_stable_raw_execution_ordinal_outside_typed_ui_subset() {
        let mut mem = exact_fixture();
        mem.put_u32(ARMY + ORDERS_GROUP_OFF_EXECUTION_TYPE, 4);
        let carrier = read_complete_army_orders_groups(&mem).unwrap();
        assert_eq!(carrier.armies[0].execution_type, 4);

        let mut payload = Vec::new();
        append_orders_group_extension(&mut payload, &carrier).unwrap();
        assert_eq!(
            decode_orders_group_extension(&payload).unwrap().armies[0].execution_type,
            4
        );
    }

    #[test]
    fn rejects_bounded_scalar_outside_exact_ui_enum() {
        let mut mem = exact_fixture();
        mem.put_u32(ARMY + ORDERS_GROUP_OFF_COHESION_TYPE, 4);
        assert_eq!(
            read_complete_army_orders_groups(&mem),
            Err(OrdersGroupReadError::InvalidObject {
                army_id: 17,
                reason: "cohesion type outside exact UI enum",
            })
        );
    }

    #[test]
    fn rejects_unknown_type_53_object_vtable() {
        let mut mem = exact_fixture();
        mem.put_u64(ARMY_GROUP, MODULE + 0x1234);
        assert_eq!(
            read_complete_army_orders_groups(&mem),
            Err(OrdersGroupReadError::InvalidObject {
                army_id: 23,
                reason: "unknown type-53 vtable",
            })
        );
    }

    #[test]
    fn pointer_free_extension_round_trips_complete_manager_and_scalars() {
        let carrier = read_complete_army_orders_groups(&exact_fixture()).unwrap();
        let mut payload = Vec::new();
        append_orders_group_extension(&mut payload, &carrier).unwrap();
        assert_eq!(
            decode_orders_group_extension(&payload).unwrap(),
            DecodedOrdersGroupExtension {
                owner_colours: carrier.owner_colours,
                complete_manager: true,
                owner_identities_complete: true,
                army_group_count: 1,
                army_group_ids: vec![23],
                registered_entry_count: 2,
                armies: carrier.armies,
            }
        );
        assert!(!payload.windows(8).any(|bytes| {
            u64::from_le_bytes(bytes.try_into().unwrap()) == ARMY
                || u64::from_le_bytes(bytes.try_into().unwrap()) == ARMY_GROUP
        }));
    }

    #[test]
    fn old_owner_census_versions_are_refused_without_fallback() {
        let carrier = read_complete_army_orders_groups(&exact_fixture()).unwrap();
        let mut payload = Vec::new();
        append_orders_group_extension(&mut payload, &carrier).unwrap();
        for version in [1u16, 2] {
            payload[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(decode_orders_group_extension(&payload).unwrap_err().contains("unsupported version"));
        }
    }
    #[test]
    fn colours_are_sampled_once_per_typed_owner_without_orders_and_keep_read_failures_local() {
        let mut mem = exact_fixture();
        let before = read_complete_army_orders_groups(&mem).unwrap();
        assert!(before.owner_colours.iter().all(|row| row.status == OwnerColourStatus::Unreadable));
        let words = [0x80000000u32, 1, 0x3f000000, 0x3f800000];
        mem.put(ARMY + 0x110, &words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>());
        let after = read_complete_army_orders_groups(&mem).unwrap();
        assert_eq!(before.armies, after.armies);
        assert_eq!(before.army_group_ids, after.army_group_ids);
        assert_eq!(after.owner_colours.len(), 2);
        assert_eq!(after.owner_colours[0], OwnerColourRow { owner_kind: 0, owner_id: 17,
            status: OwnerColourStatus::Exact, rgba_f32_bits: words });
        assert_eq!(after.owner_colours[1].status, OwnerColourStatus::Unreadable);
        let invalid = [0x7fc01234u32, 0, 0, 0x3f800000];
        mem.put(ARMY_GROUP + 0x110, &invalid.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>());
        let carrier = read_complete_army_orders_groups(&mem).unwrap();
        assert_eq!(carrier.owner_colours[1].status, OwnerColourStatus::NonFinite);
        assert_eq!(carrier.owner_colours[1].rgba_f32_bits, invalid);
        let mut bytes = vec![];
        append_orders_group_extension(&mut bytes, &carrier).unwrap();
        assert_eq!(decode_orders_group_extension(&bytes).unwrap().owner_colours, carrier.owner_colours);
    }

    #[test]
    fn disagreeing_colour_reads_refuse_only_that_owner() {
        struct Changing { memory: TestMemory, reads: std::cell::Cell<u32> }
        impl BattleplansDepthMemory for Changing {
            fn module_base(&self) -> u64 { MODULE }
            fn read_u64(&self, a: u64) -> Option<u64> { self.memory.read_u64(a) }
            fn read_u32(&self, a: u64) -> Option<u32> { self.memory.read_u32(a) }
            fn read_i32(&self, a: u64) -> Option<i32> { self.memory.read_i32(a) }
            fn is_heap_pointer(&self, a: u64) -> bool { self.memory.is_heap_pointer(a) }
            fn read_bytes(&self, a: u64, n: usize) -> Option<Vec<u8>> {
                if a == ARMY + 0x110 && n == 16 {
                    let read = self.reads.get(); self.reads.set(read + 1);
                    let mut words = vec![0; 16]; words[0] = read as u8; Some(words)
                } else { self.memory.read_bytes(a, n) }
            }
        }
        let memory = Changing { memory: exact_fixture(), reads: std::cell::Cell::new(0) };
        assert_eq!(read_complete_army_orders_group_roots(&memory).unwrap().roots.len(), 2);
        assert_eq!(memory.reads.get(), 0, "D27 root discovery does not sample D54 colour");
        let carrier = read_complete_army_orders_groups(&memory).unwrap();
        assert_eq!(memory.reads.get(), 2);
        assert_eq!(carrier.registered_entry_count, 2);
        assert_eq!(carrier.owner_colours[0].status, OwnerColourStatus::Unstable);
        assert_eq!(carrier.owner_colours[0].rgba_f32_bits, [0; 4]);
        assert_eq!(carrier.owner_colours[1].status, OwnerColourStatus::Unreadable);
    }

}
