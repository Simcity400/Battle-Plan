
use engine_sites::Rva;
use crate::{
    Memory, allocation_equipment_row, carmy, ccountry, cequipment_variant,
    cpdx_array::Array,
    cproduction_status, equipment_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::{Buffer, Outcome},
};

pub use equipment_entry::Row;

pub const VTABLE_RVA: Rva = Rva(0x27c6ec8);
pub const ALLOCATIONS: usize = 0x08;
pub const ITEMS: usize = 0x20;
pub const CAPACITY: usize = 0x28;
pub const COUNT: usize = 0x2c;
pub const ALLOCATOR: usize = 0x30;
pub const ALLOW_ZERO: usize = 0x38;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StockpileRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionSource {
    pub pointer: usize,
    pub division: u64,
    pub template: u64,
    pub country: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionRow {
    pub division: u64,
    pub template: u64,
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}

pub fn entries(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
) -> Result<Array, &'static str> {
    authenticated_array(memory, module_base, pool, ITEMS, equipment_entry::STRIDE)
}

pub fn model_selection(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
) -> Result<(usize, u64), &'static str> {
    model_selection_observed(memory, module_base, pool, &mut 0)
}

pub fn model_selection_observed(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
    observed_flags: &mut u64,
) -> Result<(usize, u64), &'static str> {
    model_selection_entries(memory, module_base, pool, observed_flags)
}

fn model_selection_entries(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
    observed_flags: &mut u64,
) -> Result<(usize, u64), &'static str> {
    *observed_flags = 0;
    let expected = engine_sites::at(module_base, VTABLE_RVA)
        .ok_or("equipment pool vtable address overflow")?;
    if read::<u64>(memory, pool) != Some(expected as u64) {
        return Err("equipment pool vtable unreadable or mismatch");
    }
    let header_at = pool
        .checked_add(ITEMS)
        .ok_or("equipment pool address overflow")?;
    let header = read::<[u8; 16]>(memory, header_at).ok_or("equipment pool array unreadable")?;
    let array = Array::from_header(header, equipment_entry::STRIDE)
        .ok_or("equipment pool array invalid")?;
    const BATCH: usize = 64;
    let mut bytes = [0; BATCH * equipment_entry::STRIDE];
    let mut selected = 0;
    let mut positive_flags = 0u64;
    let mut largest = None;
    let mut first = 0_u32;
    while first < array.count {
        let count = ((array.count - first) as usize).min(BATCH);
        let address = array
            .element(first, equipment_entry::STRIDE)
            .ok_or("equipment model pool address overflow")?;
        let copied = &mut bytes[..count * equipment_entry::STRIDE];
        if !crate::read_slice(memory, address, copied) {
            return Err("equipment model pool entries unreadable");
        }
        let mut hints = copied.chunks_exact(equipment_entry::STRIDE);
        let mut hinting = hints
            .by_ref()
            .take(4)
            .all(|entry| hint_model_variant(entry, crate::prefetch::span));
        for entry in copied.chunks_exact(equipment_entry::STRIDE) {
            let amount = i64::from_le_bytes(entry[8..16].try_into().unwrap());
            let variant = u64::from_le_bytes(entry[..8].try_into().unwrap()) as usize;
            if amount > 0 {
                let flags = transport_variant_flags(memory, module_base, variant)?;
                positive_flags |= flags;
                *observed_flags = positive_flags;
            }
            if largest.is_none_or(|previous| amount > previous) {
                largest = Some(amount);
                selected = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                    .map_err(|_| "equipment model variant pointer overflow")?;
            }
            if hinting && let Some(next) = hints.next() {
                hinting = hint_model_variant(next, crate::prefetch::span);
            }
        }
        first += count as u32;
    }
    Ok((selected, positive_flags))
}

fn transport_variant_flags(
    memory: &impl Memory,
    module_base: usize,
    variant: usize,
) -> Result<u64, &'static str> {
    if variant == 0 {
        return Err("positive ship equipment has null variant");
    }
    let identity =
        read::<[u8; 16]>(memory, variant).ok_or("ship equipment variant identity unreadable")?;
    cequipment_variant::identity_from_header(
        engine_sites::at(module_base, cequipment_variant::VTABLE_RVA)
            .ok_or("ship equipment module overflow")?,
        identity,
    )?;
    let at = variant
        .checked_add(cequipment_variant::CLASSIFIER_FLAGS)
        .ok_or("ship equipment flag address overflow")?;
    read(memory, at).ok_or("ship equipment flags unreadable")
}

#[inline]
fn hint_model_variant(entry: &[u8], mut hint: impl FnMut(usize, usize)) -> bool {
    if i64::from_le_bytes(entry[8..16].try_into().unwrap()) <= 0 {
        return true;
    }
    let variant = u64::from_le_bytes(entry[..8].try_into().unwrap()) as usize;
    if variant == 0
        || variant.checked_add(16).is_none()
        || !crate::memory::is_canonical_range(variant, 16)
    {
        return false;
    }
    hint(variant, 16);
    let Some(flags) = variant
        .checked_add(cequipment_variant::CLASSIFIER_FLAGS)
        .filter(|&address| {
            address.checked_add(8).is_some() && crate::memory::is_canonical_range(address, 8)
        })
    else {
        return false;
    };
    hint(flags, 8);
    true
}

pub fn allocation_entries(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
) -> Result<Array, &'static str> {
    authenticated_array(
        memory,
        module_base,
        pool,
        ALLOCATIONS,
        allocation_equipment_row::STRIDE,
    )
}

fn authenticated_array(
    memory: &impl Memory,
    module_base: usize,
    pool: usize,
    field: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    let expected = engine_sites::at(module_base, VTABLE_RVA)
        .ok_or("equipment pool vtable address overflow")?;
    let actual = read::<u64>(memory, pool).ok_or("equipment pool vtable unreadable")?;
    if actual != expected as u64 {
        return Err("equipment pool vtable mismatch");
    }
    let header = pool
        .checked_add(field)
        .ok_or("equipment pool address overflow")?;
    Array::read(memory, header, stride).ok_or("equipment pool array invalid or unreadable")
}

pub fn walk(memory: &impl Memory, module_base: usize, pool: usize, output: &mut [Row]) -> Outcome {
    match entries(memory, module_base, pool) {
        Ok(array) => equipment_entry::walk(memory, module_base, array, output),
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn count_stockpiles(memory: &impl Memory, module_base: usize, game_state: usize) -> Outcome {
    walk_stockpiles_buffer(memory, module_base, game_state, crate::walk::Buffer::Count)
}
#[cfg(test)]
pub(crate) fn walk_stockpiles(memory: &impl Memory, module_base: usize, game_state: usize, output: &mut Vec<StockpileRow>) -> Outcome {
    walk_stockpiles_buffer(memory, module_base, game_state, crate::walk::Buffer::Growing(output))
}


pub fn walk_stockpiles_buffer(
    memory: &impl Memory,
    module_base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, StockpileRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, module_base, game_state)?;
        let mut found_country = false;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let manager = cproduction_status::manager(memory, module_base, country)?;
            let pool = manager
                .checked_add(cproduction_status::STOCKPILE_POOL)
                .ok_or("stockpile pool address overflow")?;
            let array = entries(memory, module_base, pool)?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("stockpile row count overflow")?,
            )?;
            let available = (output.len() - len).min(array.count as usize);
            for ordinal in 0..available {
                let address = array
                    .element(ordinal as u32, equipment_entry::STRIDE)
                    .ok_or("stockpile entry address overflow")?;
                let row = equipment_entry::read(memory, module_base, address, ordinal as u32)?;
                output[len] = StockpileRow {
                    country: country.id,
                    ordinal: row.source_ordinal,
                    variant_handle: row.variant_handle,
                    amount_raw: row.amount_raw,
                };
                len += 1;
            }
            unread_rows = unread_rows
                .checked_add(array.count - available as u32)
                .ok_or("stockpile unread row count overflow")?;
        }
        if !found_country {
            Err("stockpile country census contains no countries")
        } else {
            Ok(complete(len, unread_rows))
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn walk_divisions(
    memory: &impl Memory,
    module_base: usize,
    divisions: &[DivisionSource],
    output: &mut Vec<DivisionRow>,
) -> Outcome {
    walk_divisions_where(memory, module_base, divisions, |_| true, output)
}

pub fn count_divisions(
    memory: &impl Memory,
    module_base: usize,
    divisions: &[DivisionSource],
) -> Outcome {
    walk_divisions_buffer(memory, module_base, divisions, |_| true, Buffer::Count)
}

pub fn walk_divisions_where(
    memory: &impl Memory,
    module_base: usize,
    divisions: &[DivisionSource],
    keep: impl Fn(&DivisionSource) -> bool,
    output: &mut Vec<DivisionRow>,
) -> Outcome {
    walk_divisions_buffer(
        memory,
        module_base,
        divisions,
        keep,
        Buffer::Growing(output),
    )
}

fn walk_divisions_buffer(
    memory: &impl Memory,
    module_base: usize,
    divisions: &[DivisionSource],
    keep: impl Fn(&DivisionSource) -> bool,
    mut output: Buffer<'_, DivisionRow>,
) -> Outcome {
    let result = (|| {
        const BATCH: usize = 64;
        let mut addresses = [0_usize; BATCH];
        let mut entry_bytes = [0_u8; BATCH * equipment_entry::STRIDE];
        let mut identities = [0_u8; BATCH * 16];
        let relocated = engine_sites::at(module_base, cequipment_variant::VTABLE_RVA);
        let mut len = 0usize;
        let mut unread_rows = 0_u32;
        for division in divisions.iter().filter(|division| keep(division)) {
            let pool = division
                .pointer
                .checked_add(carmy::EQUIPMENT_POOL)
                .ok_or("division equipment pool address overflow")?;
            let array = entries(memory, module_base, pool)?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("division equipment sizing overflow")?,
            )?;
            let available = (output.len() - len).min(array.count as usize);
            let mut first = 0;
            while first < available {
                let count = (available - first).min(BATCH);
                let mut overflow = None;
                for (slot, address) in addresses[..count].iter_mut().enumerate() {
                    match array.element((first + slot) as u32, equipment_entry::STRIDE) {
                        Some(element) => *address = element,
                        None => {
                            overflow = Some(slot);
                            break;
                        }
                    }
                }
                let ready = overflow.unwrap_or(count);
                let copied = memory.copy_records(
                    RecordSources::Pointers(&addresses[..ready]),
                    &ENTRY_LAYOUT,
                    &mut entry_bytes[..ready * equipment_entry::STRIDE],
                );
                let mut staged = 0;
                while staged < copied.min(count) {
                    let start = staged * equipment_entry::STRIDE;
                    let word = u64::from_le_bytes(
                        entry_bytes
                            [start + equipment_entry::VARIANT..start + equipment_entry::AMOUNT]
                            .try_into()
                            .unwrap(),
                    );
                    match usize::try_from(word) {
                        Ok(pointer) => addresses[staged] = pointer,
                        Err(_) => break,
                    }
                    staged += 1;
                }
                let authenticated = memory.copy_records(
                    RecordSources::Pointers(&addresses[..staged]),
                    &IDENTITY_LAYOUT,
                    &mut identities[..staged * 16],
                );
                for slot in 0..count {
                    if overflow == Some(slot) {
                        return Err("division equipment entry address overflow");
                    }
                    if slot >= copied {
                        return Err("equipment entry unreadable");
                    }
                    if slot >= staged {
                        return Err("equipment variant pointer overflow");
                    }
                    let expected = relocated.ok_or("variant module address overflow")?;
                    if slot >= authenticated {
                        return Err("variant identity unreadable");
                    }
                    let start = slot * equipment_entry::STRIDE;
                    let amount_raw = i64::from_le_bytes(
                        entry_bytes
                            [start + equipment_entry::AMOUNT..start + equipment_entry::STRIDE]
                            .try_into()
                            .unwrap(),
                    );
                    let variant_handle = cequipment_variant::identity_from_header(
                        expected,
                        identities[slot * 16..slot * 16 + 16].try_into().unwrap(),
                    )?;
                    output[len] = DivisionRow {
                        division: division.division,
                        template: division.template,
                        country: division.country,
                        ordinal: (first + slot) as u32,
                        variant_handle,
                        amount_raw,
                    };
                    len += 1;
                }
                first += count;
            }
            unread_rows = unread_rows
                .checked_add(array.count - available as u32)
                .ok_or("division equipment unread row count overflow")?;
        }
        Ok(complete(len, unread_rows))
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

const ENTRY_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: equipment_entry::STRIDE,
}];
const IDENTITY_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 16,
}];
const ENTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ENTRY_FIELD, equipment_entry::STRIDE).expect("valid record layout");
const IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&IDENTITY_FIELD, 16).expect("valid record layout");

fn complete(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests;
