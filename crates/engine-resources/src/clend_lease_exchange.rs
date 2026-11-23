
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::{Buffer, Outcome},
};
use crate::{allocation, ccountry_resources};
use engine_native::{ccountry, cequipment_variant::{self, pool as pool}};
use engine_native_parts::equipment_entry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a9_9cf8);
pub const HANDLE: usize = 0x08;
pub const RECEIVER: usize = 0xb8;
pub const SUNK: usize = 0x298;
pub const DELIVERED: usize = 0x2d8;
pub const SIZE: usize = 0x380;
pub const OUTGOING: usize = 0x758;
pub const GIVER_RESOURCES: usize = 0xb0;
pub const MONTHLY_NEED: usize = 0xd8;
pub const PERCENTAGE_NEED: usize = 0x118;
pub const ONCE_NEED: usize = 0x158;
pub const LAST_DELIVERY_DATE: usize = 0x320;
pub const ACTIVE_SINCE: usize = 0x338;
pub const FUEL_DAILY: usize = 0x350;
pub const FUEL_PERCENTAGE: usize = 0x358;
pub const FUEL_SENT: usize = 0x360;
pub const FUEL_SUNK: usize = 0x368;
pub const LAST_FUEL_DELIVERED: usize = 0x370;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub id: u64,
    pub giver: u32,
    pub receiver: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Term {
    pub variant: u64,
    pub amount_raw: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TermPool {
    pub allow_zero: bool,
    pub equipment: Vec<Term>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Terms {
    pub identity: Identity,
    pub monthly: TermPool,
    pub percentage: TermPool,
    pub once: TermPool,
    pub active_since: i32,
    pub fuel_daily_raw: i64,
    pub fuel_percentage_raw: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleHeader {
    pub counts: [u32; 3],
    pub allow_zero: [bool; 3],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleItem {
    pub mode: u8,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}

pub fn schedules(
    memory: &impl Memory,
    base: usize,
    sources: [usize; 3],
    mut header: impl FnMut(ScheduleHeader) -> Result<(), &'static str>,
    mut emit: impl FnMut(ScheduleItem) -> Result<(), &'static str>,
) -> Result<usize, &'static str> {
    let mut arrays = [Array::default(); 3];
    let mut shape = ScheduleHeader::default();
    for (index, source) in sources.into_iter().enumerate() {
        arrays[index] = pool::entries(memory, base, source)?;
        shape.counts[index] = arrays[index].count;
        shape.allow_zero[index] = engine_native_memory::read::<u8>(memory, address(source, pool::ALLOW_ZERO)?)
            .ok_or("lend-lease schedule zero policy unreadable")?
            != 0;
    }
    header(shape)?;
    let mut count = 0_usize;
    for (mode, entries) in arrays.into_iter().enumerate() {
        for ordinal in 0..entries.count {
            let item = equipment_entry::read(
                memory,
                base,
                entries
                    .element(ordinal, equipment_entry::STRIDE)
                    .ok_or("lend-lease schedule entry address overflow")?,
                ordinal,
            )?;
            emit(ScheduleItem {
                mode: mode as u8,
                ordinal,
                variant: item.variant_handle,
                amount_raw: item.amount_raw,
            })?;
            count = count
                .checked_add(1)
                .ok_or("lend-lease schedule count overflow")?;
        }
    }
    Ok(count)
}

fn address(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("lend-lease address overflow")
}

pub fn identity(
    memory: &impl Memory,
    base: usize,
    exchange: usize,
) -> Result<Identity, &'static str> {
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("lend-lease module address overflow")? as u64;
    if engine_native_memory::read::<u64>(memory, exchange) != Some(expected) {
        return Err("lend-lease exchange vtable unreadable or mismatch");
    }
    let id = engine_native_memory::read::<u64>(memory, address(exchange, HANDLE)?)
        .ok_or("lend-lease exchange reference unreadable")?;
    let resources = engine_native_memory::read::<u64>(memory, address(exchange, GIVER_RESOURCES)?)
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("lend-lease giver resources unreadable or null")?;
    let resources_vtable = engine_sites::at(base, ccountry_resources::VTABLE_RVA)
        .ok_or("lend-lease module address overflow")? as u64;
    if engine_native_memory::read::<u64>(memory, resources) != Some(resources_vtable) {
        return Err("lend-lease giver resources vtable unreadable or mismatch");
    }
    let giver = engine_native_memory::read(memory, address(resources, ccountry_resources::OWNER)?)
        .ok_or("lend-lease giver unreadable")?;
    let receiver = engine_native_memory::read(memory, address(exchange, RECEIVER)?)
        .ok_or("lend-lease receiver unreadable")?;
    Ok(Identity {
        id,
        giver,
        receiver,
    })
}

pub fn terms(
    memory: &impl Memory,
    base: usize,
    exchange: usize,
    expected: Identity,
    output: &mut Terms,
) -> Result<(), &'static str> {
    if identity(memory, base, exchange)? != expected {
        return Err("lend-lease terms identity changed");
    }
    for (offset, output) in [
        (MONTHLY_NEED, &mut output.monthly),
        (PERCENTAGE_NEED, &mut output.percentage),
        (ONCE_NEED, &mut output.once),
    ] {
        let source = address(exchange, offset)?;
        let entries = pool::entries(memory, base, source)?;
        let allow_zero = engine_native_memory::read::<u8>(memory, address(source, pool::ALLOW_ZERO)?)
            .ok_or("lend-lease terms zero-entry flag unreadable")?
            != 0;
        allocation::resize_default(&mut output.equipment, entries.count as usize)
            .map_err(|_| "lend-lease terms allocation failed")?;
        for (ordinal, output) in output.equipment.iter_mut().enumerate() {
            let entry = equipment_entry::read(
                memory,
                base,
                entries
                    .element(ordinal as u32, equipment_entry::STRIDE)
                    .ok_or("lend-lease terms entry address invalid")?,
                ordinal as u32,
            )?;
            *output = Term {
                variant: entry.variant_handle,
                amount_raw: entry.amount_raw,
            };
        }
        output.allow_zero = allow_zero;
    }
    output.active_since = engine_native_memory::read(memory, address(exchange, ACTIVE_SINCE)?)
        .ok_or("lend-lease active date unreadable")?;
    output.fuel_daily_raw = engine_native_memory::read(memory, address(exchange, FUEL_DAILY)?)
        .ok_or("lend-lease daily fuel unreadable")?;
    output.fuel_percentage_raw = engine_native_memory::read(memory, address(exchange, FUEL_PERCENTAGE)?)
        .ok_or("lend-lease fuel share unreadable")?;
    output.identity = expected;
    Ok(())
}

pub const DELIVERED_KIND: u8 = 0;
pub const SUNK_KIND: u8 = 1;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub giver: u32,
    pub exchange: u32,
    pub kind: u8,
    pub variant: u32,
    pub receiver: u32,
    pub amount_raw: i64,
}

const EXCHANGE_BYTES: usize = DELIVERED + pool::ITEMS + 16;
const _: () = assert!(EXCHANGE_BYTES <= SIZE);
const EXCHANGE_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: EXCHANGE_BYTES,
}];
const EXCHANGE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&EXCHANGE_FIELDS, EXCHANGE_BYTES).expect("valid record layout");
const BATCH: usize = 16;

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Growing(rows))
}

pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match count(memory, base, game_state) {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("lend-lease row demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: Buffer<'_, Row>,
) -> Outcome {
    let mut len = 0;
    let mut unread_rows = 0_u32;
    match walk_inner(
        memory,
        base,
        game_state,
        &mut rows,
        &mut len,
        &mut unread_rows,
    ) {
        Ok(()) if unread_rows == 0 => Outcome::Rows { len },
        Ok(()) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Buffer<'_, Row>,
    len: &mut usize,
    unread_rows: &mut u32,
) -> Result<(), &'static str> {
    let relocate = |rva| {
        engine_sites::at(base, rva)
            .map(|address| address as u64)
            .ok_or("lend-lease module address overflow")
    };
    let exchange_vtable = relocate(VTABLE_RVA)?;
    let pool_vtable = relocate(pool::VTABLE_RVA)?;
    let variant_vtable = relocate(engine_native_parts::equipment_variant_identity::VTABLE_RVA)? as usize;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut found = false;
    let mut pointer_bytes = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut exchanges = [0_u8; BATCH * EXCHANGE_BYTES];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found = true;
        let resources = ccountry_resources::resources(memory, base, &countries, country)?;
        let list = resources
            .checked_add(OUTGOING)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("lend-lease exchange list unreadable or invalid")?;
        let mut first = 0;
        while first < list.count {
            let count = ((list.count - first) as usize).min(BATCH);
            let span = list
                .slice(first, count as u32, 8)
                .map_err(|_| "lend-lease exchange window invalid")?;
            if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
                return Err("lend-lease exchange pointers unreadable");
            }
            for (pointer, bytes) in pointers.iter_mut().zip(pointer_bytes.chunks_exact(8)) {
                *pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            }
            if pointers[..count].contains(&0) {
                return Err("lend-lease exchange pointer null");
            }
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &EXCHANGE_LAYOUT,
                &mut exchanges[..count * EXCHANGE_BYTES],
            );
            for index in 0..count {
                if copied < index + 1 {
                    return Err("lend-lease exchange unreadable");
                }
                let bytes = &exchanges[index * EXCHANGE_BYTES..(index + 1) * EXCHANGE_BYTES];
                if u64_at(bytes, 0) != exchange_vtable {
                    return Err("lend-lease exchange vtable mismatch");
                }
                let exchange = (u64_at(bytes, HANDLE) >> 32) as u32;
                let receiver = u32_at(bytes, RECEIVER);
                for (kind, at) in [(DELIVERED_KIND, DELIVERED), (SUNK_KIND, SUNK)] {
                    if u64_at(bytes, at) != pool_vtable {
                        return Err("lend-lease pool vtable mismatch");
                    }
                    let header = at + pool::ITEMS;
                    let items = Array::from_header(
                        bytes[header..header + 16].try_into().unwrap(),
                        equipment_entry::STRIDE,
                    )
                    .ok_or("lend-lease pool entry array invalid")?;
                    rows.ensure(
                        len.checked_add(items.count as usize)
                            .ok_or("lend-lease row demand overflow")?,
                    )?;
                    if *unread_rows != 0 || items.count as usize > rows.len() - *len {
                        *unread_rows = unread_rows
                            .checked_add(items.count)
                            .ok_or("lend-lease unread row count exceeds u32")?;
                        continue;
                    }
                    let start = *len;
                    for entry in 0..items.count {
                        let address = items
                            .element(entry, equipment_entry::STRIDE)
                            .ok_or("lend-lease pool entry address overflow")?;
                        let mut raw = [0_u8; equipment_entry::STRIDE];
                        if !read_slice(memory, address, &mut raw) {
                            return Err("lend-lease pool entries unreadable");
                        }
                        let variant = u64_at(&raw, equipment_entry::VARIANT) as usize;
                        let mut identity = [0_u8; 16];
                        if variant == 0 || !read_slice(memory, variant, &mut identity) {
                            return Err("lend-lease pool variant unreadable");
                        }
                        let handle =
                            engine_native_parts::equipment_variant_identity::identity_from_header(variant_vtable, identity)?;
                        rows[*len] = Row {
                            giver: country.id,
                            exchange,
                            kind,
                            variant: (handle >> 32) as u32,
                            receiver,
                            amount_raw: i64::from_le_bytes(
                                raw[equipment_entry::AMOUNT..].try_into().unwrap(),
                            ),
                        };
                        *len += 1;
                    }
                    let written = &rows[start..*len];
                    if written
                        .iter()
                        .enumerate()
                        .any(|(i, row)| written[..i].iter().any(|r| r.variant == row.variant))
                    {
                        return Err("lend-lease pool names a variant twice");
                    }
                }
            }
            first += count as u32;
        }
    }
    if !found {
        return Err("lend-lease country table has no countries");
    }
    Ok(())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests;
