//! Lend-lease exchange from giver to receiver, recording delivery and loss.
//!
//! Creation finds the giver's exchange for the receiver or allocates 0x380
//! bytes and appends to both countries' exchange lists. Delivery transfers
//! convoys, accumulates delivered equipment and credits logistics series 9
//! for received and 10 for sent quantities. Loss booking moves equipment
//! from in-transit pools into the sunk pool. The giver's save contains both
//! sunk and delivered totals. These pools accumulate over the exchange
//! lifetime; ending lend-lease deletes the exchange and its totals.

use crate::{
    Memory, ccountry, ccountry_resources, cequipment_variant, cequipment_variant_pool as pool,
    cpdx_array::Array,
    equipment_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::{Buffer, Outcome},
};

/// Lend-lease exchange primary vtable.
pub const VTABLE_RVA: usize = 0x02a0_9df8;
/// The exchange's reference (u32 type, u32 id) of its CReferenceObject base.
pub const HANDLE: usize = 0x08;
/// The receiver's raw country reference.
pub const RECEIVER: usize = 0xb8;
/// Equipment sunk in transit, per variant.
pub const SUNK: usize = 0x298;
/// Equipment delivered, per variant.
pub const DELIVERED: usize = 0x2d8;
/// Allocation size.
pub const SIZE: usize = 0x380;
/// Country-resource outgoing exchange pointer array, the giver's list, with its signed count.
pub const OUTGOING: usize = 0x758;

/// Row kinds.
pub const DELIVERED_KIND: u8 = 0;
pub const SUNK_KIND: u8 = 1;

/// One variant's amount in one pool of one exchange.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The giver's raw country reference (the country whose list holds it).
    pub giver: u32,
    /// The exchange reference's id.
    pub exchange: u32,
    /// [`DELIVERED_KIND`] or [`SUNK_KIND`].
    pub kind: u8,
    /// The variant reference's id (type 70).
    pub variant: u32,
    /// The receiver's raw country reference.
    pub receiver: u32,
    /// The pool amount, fixed5.
    pub amount_raw: i64,
}

/// Exchange bytes copied: through the delivered pool's entry header.
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

/// Reads every country's outgoing exchanges in country slot and list order:
/// the delivered pool, then the sunk pool, each in pool order. Every exchange
/// and pool is authenticated by vtable and every variant by vtable and
/// reference type; a pool may name a variant once. Any other form and any
/// failed read makes the table unavailable. From the first exchange whose
/// rows do not fit, its and every later exchange's pool entries are counted
/// unread.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, rows: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Fixed(rows))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Growing(rows))
}

/// Original exchange/pool headers only, for attach and cadence sizing.
pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match walk(memory, base, game_state, &mut []) {
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
    let relocate = |rva: usize| {
        base.checked_add(rva)
            .map(|address| address as u64)
            .ok_or("lend-lease module address overflow")
    };
    let exchange_vtable = relocate(VTABLE_RVA)?;
    let pool_vtable = relocate(pool::VTABLE_RVA)?;
    let variant_vtable = relocate(cequipment_variant::VTABLE_RVA)? as usize;
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
                    // Delivery and loss retain complete equipment pools.
                    // Grow from the already acquired header.
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
                            cequipment_variant::identity_from_header(variant_vtable, identity)?;
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
