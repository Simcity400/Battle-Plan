
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};
use engine_native_air::cstrategic_air;
use crate::{sair_wing_combat_data, sdamage_entry};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_5288);
pub const FIRST_RECORDS: usize = 0x08;
#[cfg(test)]
pub const FIRST_RECORDS_COUNT: usize = 0x14;
pub const SECOND_RECORDS: usize = 0x20;
pub const OWNER: usize = 0x38;
pub const UPDATE_RECORD_RVA: Rva = Rva(0x019e_fb90);
pub const PRUNE_RVA: Rva = Rva(0x019f_07b0);

#[derive(Debug, Clone, Copy)]
pub struct Root {
    pub country: u32,
    pub region: u32,
    pub address: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub records: Outcome,
    pub damage: Outcome,
}

const ROOT_BATCH: usize = 64;
const ROOT_RECORD_BYTES: usize = 44;
const POINTER_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELD, 8).expect("valid record layout");
const ROOT_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: OWNER,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: FIRST_RECORDS,
        destination_offset: 12,
        len: 16,
    },
    RecordField {
        source_offset: SECOND_RECORDS,
        destination_offset: 28,
        len: 16,
    },
];
const ROOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ROOT_FIELDS, ROOT_RECORD_BYTES).expect("valid record layout");

pub fn walk(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    records: &mut Vec<sair_wing_combat_data::Row>,
    damage: &mut Vec<sdamage_entry::Row>,
) -> Capture {
    walk_buffer(
        memory,
        base,
        roots,
        engine_native_memory::walk::Buffer::Growing(records),
        engine_native_memory::walk::Buffer::Growing(damage),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    roots: &[Root],
    mut records: engine_native_memory::walk::Buffer<'_, sair_wing_combat_data::Row>,
    mut damage: engine_native_memory::walk::Buffer<'_, sdamage_entry::Row>,
) -> Capture {
    let result = (|| {
        let mut record_len = 0_usize;
        let mut record_omitted = 0_u32;
        let mut damage_len = 0_usize;
        let mut damage_omitted = 0_u32;
        let mut damage_error = None;
        let mut addresses = [0_usize; ROOT_BATCH];
        let mut copied_fields = [0_usize; ROOT_BATCH];
        let mut identities = [0_u8; ROOT_BATCH * ROOT_RECORD_BYTES];
        let mut batch_first = 0;
        let mut batch_len = 0;
        for (index, root) in roots.iter().enumerate() {
            if index >= batch_first + batch_len {
                batch_first = index;
                batch_len = (roots.len() - index).min(ROOT_BATCH);
                for (slot, staged) in addresses[..batch_len].iter_mut().enumerate() {
                    *staged = roots[index + slot].address;
                }
                let completed = memory.copy_records(
                    RecordSources::Pointers(&addresses[..batch_len]),
                    &ROOT_LAYOUT,
                    &mut identities[..batch_len * ROOT_RECORD_BYTES],
                );
                for (slot, fields) in copied_fields[..batch_len].iter_mut().enumerate() {
                    *fields = completed
                        .saturating_sub(slot * ROOT_FIELDS.len())
                        .min(ROOT_FIELDS.len());
                }
            }
            let slot = index - batch_first;
            let copied = copied_fields[slot];
            let record = &identities[slot * ROOT_RECORD_BYTES..(slot + 1) * ROOT_RECORD_BYTES];
            let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
            if copied == 0 || u64::from_le_bytes(record[..8].try_into().unwrap()) != expected as u64
            {
                return Err("air combat region vtable mismatch or unreadable");
            }
            root.address
                .checked_add(OWNER)
                .ok_or("air combat region address overflow")?;
            if copied < 2 {
                return Err("air combat region owner unreadable");
            }
            let region_owner = u32::from_le_bytes(record[8..12].try_into().unwrap());
            for (side, offset) in [(0_u8, FIRST_RECORDS), (1, SECOND_RECORDS)] {
                root.address
                    .checked_add(offset)
                    .ok_or("air combat region address overflow")?;
                let header = 12 + usize::from(side) * 16;
                let vector = (copied > 2 + usize::from(side))
                    .then(|| {
                        Array::from_header(
                            record[header..header + 16].try_into().unwrap(),
                            sair_wing_combat_data::STRIDE,
                        )
                    })
                    .flatten()
                    .ok_or("air combat record vector unreadable or invalid")?;
                records.ensure(
                    record_len
                        .checked_add(vector.count as usize)
                        .ok_or("air combat record count overflow")?,
                )?;
                for ordinal in 0..vector.count {
                    let address = vector
                        .element(ordinal, sair_wing_combat_data::STRIDE)
                        .ok_or("air combat record address overflow")?;
                    let need_record = record_len < records.len();
                    let need_damage = damage_error.is_none()
                        && (damage.is_growing() || damage_len < damage.len());
                    let row = if need_record || need_damage {
                        Some(sair_wing_combat_data::read_row(
                            memory,
                            base,
                            address,
                            root.country,
                            root.region,
                            side,
                            region_owner,
                        )?)
                    } else {
                        None
                    };
                    if need_record {
                        records[record_len] = row.unwrap();
                        record_len += 1;
                    } else {
                        record_omitted = record_omitted
                            .checked_add(1)
                            .ok_or("air combat record count overflow")?;
                    }
                    if damage_error.is_some() {
                        continue;
                    }
                    let damage_result = (|| {
                        for (channel, offset) in [
                            (0, sair_wing_combat_data::FIRST_DAMAGE),
                            (1, sair_wing_combat_data::SECOND_DAMAGE),
                        ] {
                            let header = address
                                .checked_add(offset)
                                .ok_or("air damage vector address overflow")?;
                            let entries = Array::read(memory, header, sdamage_entry::STRIDE)
                                .ok_or("air damage vector unreadable or invalid")?;
                            damage.ensure(
                                damage_len
                                    .checked_add(entries.count as usize)
                                    .ok_or("air damage row count overflow")?,
                            )?;
                            let available = damage.len() - damage_len;
                            let admitted = (entries.count as usize).min(available);
                            damage_omitted = damage_omitted
                                .checked_add(entries.count - admitted as u32)
                                .ok_or("air damage row count overflow")?;
                            for index in 0..admitted as u32 {
                                let entry = entries
                                    .element(index, sdamage_entry::STRIDE)
                                    .ok_or("air damage entry address overflow")?;
                                damage[damage_len] = sdamage_entry::read_row(
                                    memory,
                                    base,
                                    entry,
                                    row.unwrap().key,
                                    channel,
                                )?;
                                damage_len += 1;
                            }
                        }
                        Ok::<_, &'static str>(())
                    })();
                    if let Err(reason) = damage_result {
                        damage_error = Some(reason);
                    }
                }
            }
        }
        Ok(Capture {
            records: outcome(record_len, record_omitted),
            damage: match damage_error {
                Some(reason) => Outcome::Unavailable { reason },
                None => outcome(damage_len, damage_omitted),
            },
        })
    })();
    result.unwrap_or_else(|reason| Capture {
        records: Outcome::Unavailable { reason },
        damage: Outcome::Unavailable { reason },
    })
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub cause: u32,
    pub amount: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub amount_raw: i64,
}

pub const EQUIPMENT_REFUSALS: [&str; 5] = [
    "equipment pool vtable mismatch",
    "equipment pool array invalid or unreadable",
    "equipment entry unreadable",
    "variant identity unreadable",
    "variant vtable or reference type mismatch",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub wing_id: u32,
    pub date: i32,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: u8,
    pub classification: u8,
    pub copied: u8,
    pub attributed_country: u32,
    pub damage: [std::ops::Range<usize>; 2],
    pub equipment: Result<std::ops::Range<usize>, u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Side {
    pub count: u32,
    pub first: u32,
    pub records: Vec<Record>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Holder {
    pub owner: u32,
    pub sides: [Side; 2],
    pub damage: Vec<Damage>,
    pub equipment: Vec<Equipment>,
}

impl Holder {
    pub fn damage(&self, record: &Record, channel: usize) -> &[Damage] {
        &self.damage[record.damage[channel].clone()]
    }

    pub fn equipment(&self, record: &Record) -> Result<&[Equipment], &'static str> {
        record
            .equipment
            .clone()
            .map(|range| &self.equipment[range])
            .map_err(|index| EQUIPMENT_REFUSALS[usize::from(index)])
    }

    fn clear(&mut self) {
        self.owner = 0;
        for side in &mut self.sides {
            side.count = 0;
            side.first = 0;
            side.records.clear();
        }
        self.damage.clear();
        self.equipment.clear();
    }
}

#[derive(Default)]
pub struct Reader {
    records: Vec<u8>,
    entries: Vec<u8>,
    variants: Vec<(usize, Result<u64, u8>)>,
}

impl Reader {
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
            entries: Vec::new(),
            variants: Vec::new(),
        }
    }

    pub fn forget_variants(&mut self) {
        self.variants.clear();
    }

    fn variant(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<Result<u64, u8>, &'static str> {
        let at = match self
            .variants
            .binary_search_by_key(&pointer, |&(address, _)| address)
        {
            Ok(at) => return Ok(self.variants[at].1),
            Err(at) => at,
        };
        let identity = match engine_native_memory::read::<[u8; 16]>(memory, pointer) {
            None => Err(3),
            Some(bytes) => {
                let expected =
                    engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)
                        .ok_or("variant module address overflow")?;
                engine_native_parts::equipment_variant_identity::identity_from_header(expected, bytes)
                    .map_err(|_| 4)
            }
        };
        crate::allocation::reserve(&mut self.variants, 1)
            .map_err(|_| "air combat equipment allocation failed")?;
        self.variants.insert(at, (pointer, identity));
        Ok(identity)
    }
}

pub fn read_holder(
    memory: &impl Memory,
    base: usize,
    address: usize,
    since: Option<i32>,
) -> Result<Holder, &'static str> {
    let mut holder = Holder::default();
    read_holder_into(
        memory,
        base,
        address,
        since,
        &mut Reader::new(),
        &mut holder,
    )?;
    Ok(holder)
}

pub fn read_holder_into(
    memory: &impl Memory,
    base: usize,
    address: usize,
    since: Option<i32>,
    reader: &mut Reader,
    holder: &mut Holder,
) -> Result<(), &'static str> {
    holder.clear();
    let mut identity = [0_u8; ROOT_RECORD_BYTES];
    let copied = memory.copy_records(
        RecordSources::Pointers(&[address]),
        &ROOT_LAYOUT,
        &mut identity,
    );
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if copied == 0 || u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("air combat region vtable mismatch or unreadable");
    }
    if copied < ROOT_FIELDS.len() {
        return Err("air combat region header unreadable");
    }
    holder.owner = u32::from_le_bytes(identity[8..12].try_into().unwrap());
    for (side, header) in [12_usize, 28].into_iter().enumerate() {
        let vector = Array::from_header(
            identity[header..header + 16].try_into().unwrap(),
            sair_wing_combat_data::STRIDE,
        )
        .ok_or("air combat record vector invalid")?;
        let mut first = vector.count;
        if let Some(since) = since {
            while first > 0 {
                let record = vector
                    .element(first - 1, sair_wing_combat_data::STRIDE)
                    .ok_or("air combat record address overflow")?;
                let date = record
                    .checked_add(sair_wing_combat_data::DATE)
                    .and_then(|at| engine_native_memory::read::<i32>(memory, at))
                    .ok_or("air combat record date unreadable")?;
                if date < since {
                    break;
                }
                first -= 1;
            }
        } else {
            first = 0;
        }
        holder.sides[side].count = vector.count;
        holder.sides[side].first = first;
        let len = (vector.count - first) as usize;
        if len == 0 {
            continue;
        }
        let bytes = len
            .checked_mul(sair_wing_combat_data::STRIDE)
            .ok_or("air combat record address overflow")?;
        crate::allocation::resize(&mut reader.records, bytes, 0)
            .map_err(|_| "air combat record allocation failed")?;
        let start = vector
            .element(first, sair_wing_combat_data::STRIDE)
            .ok_or("air combat record address overflow")?;
        if !memory.copy(start, &mut reader.records[..bytes]) {
            return Err("air combat record fields unreadable");
        }
        crate::allocation::reserve(&mut holder.sides[side].records, len)
            .map_err(|_| "air combat record allocation failed")?;
        let records = std::mem::take(&mut reader.records);
        let result = records
            .chunks_exact(sair_wing_combat_data::STRIDE)
            .try_for_each(|bytes| {
                let record = read_record(memory, base, bytes, reader, holder)?;
                holder.sides[side].records.push(record);
                Ok(())
            });
        reader.records = records;
        result?;
    }
    Ok(())
}

fn read_record(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
    reader: &mut Reader,
    holder: &mut Holder,
) -> Result<Record, &'static str> {
    use sair_wing_combat_data as wing;
    let qword = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let expected = engine_sites::at(base, wing::VTABLE_RVA).ok_or("module address overflow")?;
    if qword(0) != expected as u64 {
        return Err("air combat record vtable mismatch");
    }
    let (selection, classification, copied) = (
        bytes[wing::SELECTION_FLAG],
        bytes[wing::CLASSIFICATION_FLAG],
        bytes[wing::COPIED_FLAG],
    );
    if selection > 1 || classification > 1 || copied > 1 {
        return Err("air combat record flag is not boolean");
    }
    let damage_vtable =
        engine_sites::at(base, sdamage_entry::VTABLE_RVA).ok_or("module address overflow")?;
    let mut damage: [std::ops::Range<usize>; 2] = Default::default();
    for (channel, offset) in [wing::FIRST_DAMAGE, wing::SECOND_DAMAGE]
        .into_iter()
        .enumerate()
    {
        let entries = Array::from_header(
            bytes[offset..offset + 16].try_into().unwrap(),
            sdamage_entry::STRIDE,
        )
        .ok_or("air damage vector unreadable or invalid")?;
        let len = entries.count as usize * sdamage_entry::STRIDE;
        crate::allocation::resize(&mut reader.entries, len, 0)
            .map_err(|_| "air damage allocation failed")?;
        if len != 0 && !memory.copy(entries.data, &mut reader.entries[..len]) {
            return Err("air damage entry fields unreadable");
        }
        crate::allocation::reserve(&mut holder.damage, entries.count as usize)
            .map_err(|_| "air damage allocation failed")?;
        let begin = holder.damage.len();
        for entry in reader.entries[..len].chunks_exact(sdamage_entry::STRIDE) {
            if u64::from_le_bytes(entry[..8].try_into().unwrap()) != damage_vtable as u64 {
                return Err("air damage entry vtable mismatch");
            }
            let at = sdamage_entry::CAUSE_TYPE;
            let amount = sdamage_entry::AMOUNT;
            holder.damage.push(Damage {
                cause: u32::from_le_bytes(entry[at..at + 4].try_into().unwrap()),
                amount: i64::from_le_bytes(entry[amount..amount + 8].try_into().unwrap()),
            });
        }
        damage[channel] = begin..holder.damage.len();
    }
    let equipment = equipment(memory, base, bytes, reader, holder)?;
    Ok(Record {
        wing_id: word(wing::WING_ID),
        date: word(wing::DATE) as i32,
        planes: word(wing::PLANE_COUNT) as i32,
        mission_mask: word(wing::MISSION_MASK),
        selection,
        classification,
        copied,
        attributed_country: word(wing::ATTRIBUTED_COUNTRY),
        damage,
        equipment,
    })
}

fn equipment(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
    reader: &mut Reader,
    holder: &mut Holder,
) -> Result<Result<std::ops::Range<usize>, u8>, &'static str> {
    use engine_native::cequipment_variant::pool as pool;
    use engine_native_parts::equipment_entry as entry;
    let at = sair_wing_combat_data::EQUIPMENT_POOL;
    let expected =
        engine_sites::at(base, pool::VTABLE_RVA).ok_or("equipment pool vtable address overflow")?;
    if u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) != expected as u64 {
        return Ok(Err(0));
    }
    let header = at + pool::ITEMS;
    let Some(entries) = Array::from_header(
        bytes[header..header + 16].try_into().unwrap(),
        entry::STRIDE,
    ) else {
        return Ok(Err(1));
    };
    let len = entries.count as usize * entry::STRIDE;
    crate::allocation::resize(&mut reader.entries, len, 0)
        .map_err(|_| "air combat equipment allocation failed")?;
    if len != 0 && !memory.copy(entries.data, &mut reader.entries[..len]) {
        return Ok(Err(2));
    }
    crate::allocation::reserve(&mut holder.equipment, entries.count as usize)
        .map_err(|_| "air combat equipment allocation failed")?;
    let begin = holder.equipment.len();
    let rows = std::mem::take(&mut reader.entries);
    let result = (|| {
        for row in rows[..len].chunks_exact(entry::STRIDE) {
            let variant =
                u64::from_le_bytes(row[entry::VARIANT..entry::AMOUNT].try_into().unwrap());
            let Ok(variant) = usize::try_from(variant) else {
                return Ok(Err(3));
            };
            let variant = match reader.variant(memory, base, variant)? {
                Ok(variant) => variant,
                Err(index) => return Ok(Err(index)),
            };
            holder.equipment.push(Equipment {
                variant,
                amount_raw: i64::from_le_bytes(
                    row[entry::AMOUNT..entry::STRIDE].try_into().unwrap(),
                ),
            });
        }
        Ok(Ok(begin..holder.equipment.len()))
    })();
    reader.entries = rows;
    if !matches!(result, Ok(Ok(_))) {
        holder.equipment.truncate(begin);
    }
    result
}

pub fn visit_holders(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visitor: impl FnMut(Root) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    cstrategic_air::visit(memory, base, game_state, |country| {
        holders_of(memory, country, &mut visitor)
    })
}

fn holders_of(
    memory: &impl Memory,
    country: cstrategic_air::Root,
    visitor: &mut impl FnMut(Root) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let regions = country
        .address
        .checked_add(cstrategic_air::REGION_COMBAT)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("air combat region roster unreadable or invalid")?;
    let mut slots = [0_usize; ROOT_BATCH];
    let mut slot_bytes = [0_u8; ROOT_BATCH * 8];
    let mut first = 0;
    while first < regions.count {
        let count = ((regions.count - first) as usize).min(ROOT_BATCH);
        for (index, slot) in slots[..count].iter_mut().enumerate() {
            *slot = regions.element(first + index as u32, 8).unwrap_or(0);
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&slots[..count]),
            &POINTER_LAYOUT,
            &mut slot_bytes[..count * 8],
        );
        if copied < count {
            return Err("air combat region pointer unreadable or address overflow");
        }
        for index in 0..count {
            let bytes = slot_bytes[index * 8..index * 8 + 8].try_into().unwrap();
            let address = usize::try_from(u64::from_le_bytes(bytes))
                .map_err(|_| "air combat region pointer address overflow")?;
            if address != 0 {
                visitor(Root {
                    country: country.country,
                    region: first + index as u32,
                    address,
                })?;
            }
        }
        first += count as u32;
    }
    Ok(())
}

pub fn locate(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    owner: u32,
    holder: usize,
) -> Result<Option<Root>, &'static str> {
    let canonical = engine_native::ccountry::Reader::new(memory, base, game_state)?
        .resolve(owner)?
        .canonical_slot;
    let manager = game_state
        .checked_add(engine_native::cgame_state::STRATEGIC_AIR_MANAGER)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("strategic air manager pointer unreadable or address overflow")?;
    let expected = engine_sites::at(base, engine_native_air::cstrategic_air_manager::VTABLE_RVA)
        .ok_or("strategic air manager vtable address overflow")?;
    if manager == 0 || engine_native_memory::read::<u64>(memory, manager) != Some(expected as u64) {
        return Err("strategic air manager vtable unreadable or mismatched");
    }
    let slots = manager
        .checked_add(engine_native_air::cstrategic_air_manager::COUNTRY_AIR)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("strategic air country roster unreadable or invalid")?;
    let Some(slot) = slots.element(canonical, 8) else {
        return Ok(None);
    };
    let air = engine_native_memory::read::<u64>(memory, slot)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("strategic air country pointer unreadable or address overflow")?;
    if air == 0 {
        return Ok(None);
    }
    let expected = engine_sites::at(base, cstrategic_air::VTABLE_RVA)
        .ok_or("strategic air country vtable address overflow")?;
    if engine_native_memory::read::<u64>(memory, air) != Some(expected as u64) {
        return Err("strategic air country vtable unreadable or mismatched");
    }
    let country = air
        .checked_add(cstrategic_air::COUNTRY)
        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
        .ok_or("strategic air country reference unreadable or address overflow")?;
    if country != owner {
        return Ok(None);
    }
    let mut found = None;
    holders_of(
        memory,
        cstrategic_air::Root {
            address: air,
            country,
            canonical_slot: canonical,
        },
        &mut |root| {
            if root.address == holder && found.is_none() {
                found = Some(root);
            }
            Ok(())
        },
    )?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use engine_native::{ccountry, cgame_state};
    use engine_native_air::cstrategic_air_manager;

    const MEMORY_BASE: usize = 0x1000;
    const IMAGE_BASE: usize = 0x1_4000_0000;
    const REGION: usize = 0x1000;
    const FIRST: usize = 0x1200;
    const SECOND: usize = 0x1500;
    const DAMAGE: usize = 0x1800;
    const GAME: usize = 0x3000;
    const COUNTRY: usize = 0x5000;
    const MANAGER: usize = 0x6000;
    const AIR: usize = 0x7000;
    const AIR_COUNTRIES: usize = 0x8200;
    const REGION_SLOTS: usize = 0x9000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let start = address - MEMORY_BASE;
        bytes[start..start + value.len()].copy_from_slice(value);
    }

    fn vector(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        put(bytes, address, &(data as u64).to_le_bytes());
        put(bytes, address + 8, &count.to_le_bytes());
        put(bytes, address + 12, &count.to_le_bytes());
    }

    #[test]
    fn a_mid_batch_root_fault_keeps_its_reason_and_leaves_later_roots_unused() {
        struct Refuse<'a> {
            memory: SliceMemory<'a>,
            address: usize,
        }
        impl Memory for Refuse<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= self.address && self.address - address < destination.len() {
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }
        let mut bytes = vec![0; 0x1000];
        let roots: [Root; 3] = std::array::from_fn(|index| {
            let address = REGION + index * 0x100;
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(&mut bytes, address + OWNER, &(index as u32).to_le_bytes());
            Root {
                country: 3,
                region: index as u32,
                address,
            }
        });
        assert_eq!(
            walk(
                &SliceMemory {
                    base: MEMORY_BASE,
                    bytes: &bytes
                },
                IMAGE_BASE,
                &roots,
                &mut Vec::new(),
                &mut Vec::new()
            ),
            Capture {
                records: Outcome::Rows { len: 0 },
                damage: Outcome::Rows { len: 0 },
            }
        );
        for (address, reason) in [
            (
                REGION + 0x100,
                "air combat region vtable mismatch or unreadable",
            ),
            (REGION + 0x100 + OWNER, "air combat region owner unreadable"),
            (
                REGION + 0x100 + SECOND_RECORDS,
                "air combat record vector unreadable or invalid",
            ),
        ] {
            let memory = Refuse {
                memory: SliceMemory {
                    base: MEMORY_BASE,
                    bytes: &bytes,
                },
                address,
            };
            assert_eq!(
                walk(&memory, IMAGE_BASE, &roots, &mut Vec::new(), &mut Vec::new()),
                Capture {
                    records: Outcome::Unavailable { reason },
                    damage: Outcome::Unavailable { reason },
                },
                "refused {address:#x}"
            );
        }
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        put(
            &mut bytes,
            REGION,
            &((IMAGE_BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, REGION + OWNER, &77_u32.to_le_bytes());
        vector(&mut bytes, REGION + FIRST_RECORDS, FIRST, 2);
        vector(&mut bytes, REGION + SECOND_RECORDS, SECOND, 1);
        for (address, wing, classification) in [
            (FIRST, 9_u32, 0),
            (FIRST + sair_wing_combat_data::STRIDE, 9_u32, 1),
            (SECOND, 10_u32, 0),
        ] {
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + sair_wing_combat_data::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::WING_ID,
                &wing.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::DATE,
                &(-3_i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::PLANE_COUNT,
                &i32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::MISSION_MASK,
                &u32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sair_wing_combat_data::CLASSIFICATION_FLAG,
                &[classification],
            );
        }
        vector(
            &mut bytes,
            FIRST + sair_wing_combat_data::FIRST_DAMAGE,
            DAMAGE,
            2,
        );
        for (index, cause, amount) in [(0, u32::MAX, i64::MAX), (1, 1, i64::MIN)] {
            let address = DAMAGE + index * sdamage_entry::STRIDE;
            put(
                &mut bytes,
                address,
                &((IMAGE_BASE + sdamage_entry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sdamage_entry::CAUSE_TYPE,
                &cause.to_le_bytes(),
            );
            put(
                &mut bytes,
                address + sdamage_entry::AMOUNT,
                &amount.to_le_bytes(),
            );
        }
        bytes
    }

    fn world_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        bytes.resize(0x9000, 0);
        vector(&mut bytes, GAME + cgame_state::COUNTRIES, 0x8000, 3);
        vector(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, 0x8100, 4);
        put(&mut bytes, 0x8008, &(COUNTRY as u64).to_le_bytes());
        put(&mut bytes, 0x8010, &(COUNTRY as u64).to_le_bytes());
        for index in 1..4 {
            put(&mut bytes, 0x8100 + index * 4, &1_u32.to_le_bytes());
        }
        put(
            &mut bytes,
            COUNTRY,
            &((IMAGE_BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            GAME + cgame_state::STRATEGIC_AIR_MANAGER,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER,
            &((IMAGE_BASE + cstrategic_air_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        vector(
            &mut bytes,
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            AIR_COUNTRIES,
            3,
        );
        put(&mut bytes, AIR_COUNTRIES + 8, &(AIR as u64).to_le_bytes());
        put(
            &mut bytes,
            AIR,
            &((IMAGE_BASE + cstrategic_air::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AIR + cstrategic_air::COUNTRY,
            &2_u32.to_le_bytes(),
        );
        vector(
            &mut bytes,
            AIR + cstrategic_air::REGION_COMBAT,
            REGION_SLOTS,
            3,
        );
        put(&mut bytes, REGION_SLOTS, &(REGION as u64).to_le_bytes());
        bytes
    }

    fn capture(
        bytes: &[u8],
        records: &mut Vec<sair_wing_combat_data::Row>,
        damage: &mut Vec<sdamage_entry::Row>,
    ) -> Capture {
        walk(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes,
            },
            IMAGE_BASE,
            &[Root {
                country: 12,
                region: 35,
                address: REGION,
            }],
            records,
            damage,
        )
    }

    #[test]
    fn retains_native_composite_identity_flags_owner_and_full_damage_width() {
        let mut bytes = fixture();
        let mut records = vec![sair_wing_combat_data::Row::default(); 3];
        let mut damage = vec![sdamage_entry::Row::default(); 2];
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Rows { len: 3 },
                damage: Outcome::Rows { len: 2 },
            }
        );
        assert_eq!(
            records[0].key,
            sair_wing_combat_data::Key {
                country: 12,
                region: 35,
                side: 0,
                date: -3,
                wing_id: 9,
                classification: 0,
            }
        );
        assert_ne!(records[0].key, records[1].key);
        assert_eq!(records[2].key.side, 1);
        assert_eq!(records[0].region_owner, 77);
        assert_eq!(records[0].attributed_country, 0);
        assert_eq!(records[0].planes, i32::MAX);
        assert_eq!(records[0].mission_mask, u32::MAX);
        assert_eq!((damage[0].cause, damage[0].amount), (u32::MAX, i64::MAX));
        assert_eq!((damage[1].cause, damage[1].amount), (1, i64::MIN));
        assert_eq!(damage[0].record, records[0].key);
        let key = records[0].key;
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::SELECTION_FLAG,
            &[1],
        );
        put(&mut bytes, FIRST + sair_wing_combat_data::COPIED_FLAG, &[1]);
        capture(&bytes, &mut records, &mut damage);
        assert_eq!(records[0].key, key);
        assert_eq!((records[0].selection, records[0].copied), (1, 1));
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::WING_ID,
            &0_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut records, &mut damage).records,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(records[0].key.wing_id, 0);
    }

    #[test]
    fn damage_fault_does_not_erase_good_record_rows_and_bad_keys_fail_both_tables() {
        let mut bytes = fixture();
        let mut records = vec![sair_wing_combat_data::Row::default(); 3];
        let mut damage = vec![sdamage_entry::Row::default(); 2];
        put(&mut bytes, DAMAGE, &0_u64.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Rows { len: 3 },
                damage: Outcome::Unavailable {
                    reason: "air damage entry vtable mismatch"
                },
            }
        );
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::CLASSIFICATION_FLAG,
            &[2],
        );
        assert_eq!(
            capture(&bytes, &mut records, &mut damage),
            Capture {
                records: Outcome::Unavailable {
                    reason: "air combat record flag is not boolean"
                },
                damage: Outcome::Unavailable {
                    reason: "air combat record flag is not boolean"
                },
            }
        );
    }

    #[test]
    fn empty_root_census_is_empty_and_malformed_vector_is_not() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, IMAGE_BASE, &[], &mut Vec::new(), &mut Vec::new()),
            Capture {
                records: Outcome::Rows { len: 0 },
                damage: Outcome::Rows { len: 0 },
            }
        );
        let mut bytes = bytes;
        put(
            &mut bytes,
            REGION + FIRST_RECORDS_COUNT,
            &3_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut Vec::new(), &mut Vec::new()),
            Capture {
                records: Outcome::Unavailable {
                    reason: "air combat record vector unreadable or invalid"
                },
                damage: Outcome::Unavailable {
                    reason: "air combat record vector unreadable or invalid"
                },
            }
        );
    }

    #[test]
    fn growing_regional_history_and_damage_use_original_mod_sized_headers() {
        let mut bytes = fixture();
        let record = bytes
            [FIRST - MEMORY_BASE..FIRST - MEMORY_BASE + sair_wing_combat_data::STRIDE]
            .to_vec();
        let damage =
            bytes[DAMAGE - MEMORY_BASE..DAMAGE - MEMORY_BASE + sdamage_entry::STRIDE].to_vec();
        let record_data = 0x10000;
        let damage_data = 0x300000;
        let mut records = Vec::new();
        let mut damages = Vec::new();
        for count in [5_000_u32, 12_000] {
            bytes.resize(
                damage_data - MEMORY_BASE + count as usize * sdamage_entry::STRIDE,
                0,
            );
            vector(&mut bytes, REGION + FIRST_RECORDS, record_data, count);
            vector(&mut bytes, REGION + SECOND_RECORDS, 0, 0);
            for index in 0..count as usize {
                let at = record_data + index * sair_wing_combat_data::STRIDE;
                bytes[at - MEMORY_BASE..at - MEMORY_BASE + sair_wing_combat_data::STRIDE]
                    .copy_from_slice(&record);
                vector(
                    &mut bytes,
                    at + sair_wing_combat_data::FIRST_DAMAGE,
                    damage_data + index * sdamage_entry::STRIDE,
                    1,
                );
                let damage_at = damage_data + index * sdamage_entry::STRIDE;
                bytes[damage_at - MEMORY_BASE..damage_at - MEMORY_BASE + sdamage_entry::STRIDE]
                    .copy_from_slice(&damage);
            }
            let memory = SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            };
            let roots = [Root {
                country: 12,
                region: 35,
                address: REGION,
            }];
            assert_eq!(
                walk_buffer(
                    &memory,
                    IMAGE_BASE,
                    &roots,
                    engine_native_memory::walk::Buffer::Count,
                    engine_native_memory::walk::Buffer::Count,
                )
                .records,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: count
                }
            );
            assert_eq!(
                walk(&memory, IMAGE_BASE, &roots, &mut records, &mut damages),
                Capture {
                    records: Outcome::Rows {
                        len: count as usize
                    },
                    damage: Outcome::Rows {
                        len: count as usize
                    }
                }
            );
        }
    }

    #[test]
    fn a_holder_read_takes_the_dated_suffix_or_every_record() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST + sair_wing_combat_data::STRIDE + sair_wing_combat_data::DATE,
            &5_i32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        let tail = read_holder(&memory, IMAGE_BASE, REGION, Some(0)).unwrap();
        assert_eq!(tail.owner, 77);
        assert_eq!((tail.sides[0].count, tail.sides[0].first), (2, 1));
        assert_eq!(tail.sides[0].records.len(), 1);
        assert_eq!(
            (
                tail.sides[0].records[0].date,
                tail.sides[0].records[0].wing_id,
                tail.sides[0].records[0].classification
            ),
            (5, 9, 1)
        );
        assert_eq!((tail.sides[1].count, tail.sides[1].first), (1, 1));
        assert!(tail.sides[1].records.is_empty());
        let whole = read_holder(&memory, IMAGE_BASE, REGION, None).unwrap();
        assert_eq!(whole.sides[0].first, 0);
        assert_eq!(whole.sides[0].records.len(), 2);
        assert_eq!(whole.sides[1].records[0].wing_id, 10);
        assert_eq!(
            whole.damage(&whole.sides[0].records[0], 0),
            [
                Damage {
                    cause: u32::MAX,
                    amount: i64::MAX
                },
                Damage {
                    cause: 1,
                    amount: i64::MIN
                },
            ]
        );
        assert!(whole.damage(&whole.sides[0].records[0], 1).is_empty());
        assert_eq!(
            whole.equipment(&whole.sides[0].records[0]),
            Err(EQUIPMENT_REFUSALS[0])
        );
        assert_eq!(
            read_holder(&memory, IMAGE_BASE, REGION + 8, None),
            Err("air combat region vtable mismatch or unreadable")
        );
    }

    #[test]
    fn equipment_entries_are_read_once_and_variants_once_per_capture() {
        use engine_native::cequipment_variant::{self as variant, pool as pool};
        const ENTRIES: usize = 0x1900;
        const VARIANT: usize = 0x1a00;
        let mut bytes = fixture();
        let at = FIRST + sair_wing_combat_data::EQUIPMENT_POOL;
        put(
            &mut bytes,
            at,
            &((IMAGE_BASE + pool::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        vector(&mut bytes, at + pool::ITEMS, ENTRIES, 2);
        for (index, amount) in [5_i64, -7].into_iter().enumerate() {
            put(
                &mut bytes,
                ENTRIES + index * 16,
                &(VARIANT as u64).to_le_bytes(),
            );
            put(&mut bytes, ENTRIES + index * 16 + 8, &amount.to_le_bytes());
        }
        let handle = (9_u64 << 32) | u64::from(variant::REFERENCE_TYPE);
        put(
            &mut bytes,
            VARIANT,
            &((IMAGE_BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, VARIANT + 8, &handle.to_le_bytes());
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        let (mut reader, mut holder) = (Reader::new(), Holder::default());
        read_holder_into(&memory, IMAGE_BASE, REGION, None, &mut reader, &mut holder).unwrap();
        let records = &holder.sides[0].records;
        assert_eq!(
            holder.equipment(&records[0]),
            Ok(&[
                Equipment {
                    variant: handle,
                    amount_raw: 5
                },
                Equipment {
                    variant: handle,
                    amount_raw: -7
                },
            ][..])
        );
        assert_eq!(reader.variants.len(), 1);
        assert_eq!(holder.equipment(&records[1]), Err(EQUIPMENT_REFUSALS[0]));
        read_holder_into(&memory, IMAGE_BASE, REGION, None, &mut reader, &mut holder).unwrap();
        assert_eq!(holder.equipment.len(), 2);
        assert_eq!(
            holder,
            read_holder(&memory, IMAGE_BASE, REGION, None).unwrap()
        );
        reader.forget_variants();
        assert!(reader.variants.is_empty());
    }

    #[test]
    fn holders_are_visited_and_located_through_their_owners_table() {
        let bytes = world_fixture();
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        let mut roots = Vec::new();
        visit_holders(&memory, IMAGE_BASE, GAME, |root| {
            roots.push((root.country, root.region, root.address));
            Ok(())
        })
        .unwrap();
        assert_eq!(roots, [(2, 0, REGION)]);
        let found = locate(&memory, IMAGE_BASE, GAME, 2, REGION)
            .unwrap()
            .unwrap();
        assert_eq!((found.country, found.region, found.address), (2, 0, REGION));
        assert!(
            locate(&memory, IMAGE_BASE, GAME, 2, REGION + 0x100)
                .unwrap()
                .is_none()
        );
    }
}
