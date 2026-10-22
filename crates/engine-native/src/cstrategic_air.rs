//! Per-country strategic-air object indexed by its manager.

use crate::{
    Memory, ccountry, cgame_state,
    cpdx_array::Array,
    cstrategic_air_manager,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0294_1ff8;
/// Per-country object allocation size.
pub const SIZE: usize = 0x1d8;
/// First air-site pointer vector visited by fuel aggregation.
pub const FIRST_AIR_SITES: usize = 0x10;
/// Second independent air-site pointer vector visited by fuel aggregation.
pub const SECOND_AIR_SITES: usize = 0x28;
/// Air-wing-pool pointer-vector data.
pub const POOLS: usize = 0x58;
/// Pool-vector count, u32.
pub const POOLS_COUNT: usize = 0x64;
/// Raw country-link index, separate from database ordinal.
pub const COUNTRY: usize = 0x90;
/// Region-indexed mission-pointer vectors with 0x18-byte headers.
/// Registration/removal maintains each list; region ID selects it.
pub const REGIONAL_MISSIONS: usize = 0x98;
/// Active mission pointer vector retained by the same two writers.
pub const ALL_ACTIVE_MISSIONS: usize = 0xb0;
/// Sorted active-mission region IDs: first ordinary registration inserts and
/// last removal erases. Carrier combat also inserts special-only regions
/// without clearing the list; see `cair_regional_mission`.
pub const ACTIVE_MISSION_REGIONS: usize = 0xc8;
/// Region-indexed loop-history pointer table, separate from statistics-kind keys.
pub const HISTORY_TABLE: usize = 0x158;
/// Region-indexed air-region-combat-data pointer table.
pub const REGION_COMBAT: usize = 0x170;
/// Per-country strategic-air constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00c3_8740;

/// An authenticated manager slot; the address is transient read context only.
#[derive(Debug, Clone, Copy)]
pub struct Root {
    pub address: usize,
    /// Current raw reference of the strategic air's country, including native aliases.
    pub country: u32,
    pub canonical_slot: u32,
}

/// Visit every nonnull country-air slot, including unexpected zero and alias
/// slots so they fail explicitly. Population uses canonical country indices;
/// null slots have no object. No allocation, retained pointer, retry, sorting
/// or call occurs. Visitor errors stop and invalidate dependent tables.
pub fn visit(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    visitor: impl FnMut(Root) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    visit_with_slots(memory, base, game_state, visitor, |_| {})
}

/// Original manager observations, before numerical transport projection.
#[derive(Debug, Clone, Copy)]
pub enum SlotObservation {
    Domain(u32),
    Slot { slot: u32, root: Option<Root> },
}

/// The same physical root pass, exposing nulls without a second census.
pub fn visit_with_slots(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visitor: impl FnMut(Root) -> Result<(), &'static str>,
    mut observer: impl FnMut(SlotObservation),
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let manager = game_state
        .checked_add(cgame_state::STRATEGIC_AIR_MANAGER)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("strategic air manager pointer unreadable or address overflow")?;
    let expected = base
        .checked_add(cstrategic_air_manager::VTABLE_RVA)
        .ok_or("strategic air manager vtable address overflow")?;
    if manager == 0 || read::<u64>(memory, manager) != Some(expected as u64) {
        return Err("strategic air manager vtable unreadable or mismatched");
    }
    let slots = manager
        .checked_add(cstrategic_air_manager::COUNTRY_AIR)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("strategic air country roster unreadable or invalid")?;
    if slots.count == 0 {
        return Err("strategic air manager has no country slots");
    }
    observer(SlotObservation::Domain(slots.count));
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("strategic air country vtable address overflow")?;
    // Slot pointers and the identity of every live object in a batch are
    // gathered in two guarded rounds instead of three per country slot. Each
    // slot is still checked and visited in order with its own copied prefix,
    // so the first reason and the visitor's position are unchanged.
    const BATCH: usize = 64;
    let mut pointers = [0_usize; BATCH];
    let mut slot_bytes = [0_u8; BATCH * 8];
    let mut live = [0_usize; BATCH];
    let mut identities = [0_u8; BATCH * 12];
    let mut first = 0;
    while first < slots.count {
        let count = ((slots.count - first) as usize).min(BATCH);
        for (index, pointer) in pointers[..count].iter_mut().enumerate() {
            *pointer = slots.element(first + index as u32, 8).unwrap_or(0);
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &SLOT_LAYOUT,
            &mut slot_bytes[..count * 8],
        );
        let mut live_len = 0;
        for index in 0..copied.min(count) {
            let bytes = slot_bytes[index * 8..index * 8 + 8].try_into().unwrap();
            let Ok(address) = usize::try_from(u64::from_le_bytes(bytes)) else {
                break;
            };
            if address != 0 {
                live[live_len] = address;
                pointers[index] = address;
                live_len += 1;
            } else {
                pointers[index] = 0;
            }
        }
        let identified = memory.copy_records(
            RecordSources::Pointers(&live[..live_len]),
            &IDENTITY_LAYOUT,
            &mut identities[..live_len * 12],
        );
        let mut live_index = 0;
        for index in 0..count {
            let slot = first + index as u32;
            if index >= copied
                || usize::try_from(u64::from_le_bytes(
                    slot_bytes[index * 8..index * 8 + 8].try_into().unwrap(),
                ))
                .is_err()
            {
                return Err("strategic air country pointer unreadable or address overflow");
            }
            let address = pointers[index];
            if address == 0 {
                observer(SlotObservation::Slot { slot, root: None });
                continue;
            }
            let record = &identities[live_index * 12..live_index * 12 + 12];
            let fields = identified
                .saturating_sub(live_index * IDENTITY_FIELDS.len())
                .min(IDENTITY_FIELDS.len());
            live_index += 1;
            if fields == 0 || u64::from_le_bytes(record[..8].try_into().unwrap()) != expected as u64
            {
                return Err("strategic air country vtable unreadable or mismatched");
            }
            if fields < IDENTITY_FIELDS.len() {
                return Err("strategic air country reference unreadable or address overflow");
            }
            let country = u32::from_le_bytes(record[8..12].try_into().unwrap());
            if countries.resolve(country)?.canonical_slot != slot {
                return Err("strategic air owner differs from canonical country slot");
            }
            let root = Root {
                address,
                country,
                canonical_slot: slot,
            };
            observer(SlotObservation::Slot {
                slot,
                root: Some(root),
            });
            visitor(root)?;
        }
        first += count as u32;
    }
    Ok(())
}

const SLOT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];

const IDENTITY_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: COUNTRY,
        destination_offset: 8,
        len: 4,
    },
];

const SLOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SLOT_FIELD, 8).expect("valid record layout");
const IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&IDENTITY_FIELDS, 12).expect("valid record layout");
