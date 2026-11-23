
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    country_identity::Country,
    cpdx_array::Array,
    read,
    walk::Outcome,
};
use engine_native::ccountry::{self, Reader};
use crate::ccountry_collaboration_data::{self, Row};

pub const VTABLE_RVA: Rva = Rva(0x02a3_7748);
pub const RELATIONS: usize = 0x28;
#[cfg(test)]
pub const RELATION_COUNT: usize = 0x34;
pub const OWNER: usize = 0x40;
pub const INCOMING_COUNTRIES: usize = 0x48;

#[derive(Clone, Copy)]
struct Status {
    outgoing: Array,
    incoming: Array,
}

fn status(memory: &impl Memory, base: usize, country: Country) -> Result<Status, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::COLLABORATION_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("collaboration status pointer unreadable or null")?;
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    let owner_address = pointer
        .checked_add(OWNER)
        .ok_or("collaboration status address overflow")?;
    let mut actual_vtable = [0; 8];
    let mut actual_owner = [0; 8];
    let completed = memory.copy_scattered(
        [pointer, owner_address],
        &mut [&mut actual_vtable, &mut actual_owner],
    );
    if completed < 1 || u64::from_le_bytes(actual_vtable) != vtable as u64 {
        return Err("collaboration status vtable mismatch or unreadable");
    }
    if completed < 2 || u64::from_le_bytes(actual_owner) != country.pointer as u64 {
        return Err("collaboration status owner mismatch or unreadable");
    }
    let outgoing = pointer
        .checked_add(RELATIONS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("collaboration outgoing array unreadable or invalid")?;
    let incoming = pointer
        .checked_add(INCOMING_COUNTRIES)
        .and_then(|address| Array::read(memory, address, 4))
        .ok_or("collaboration incoming array unreadable or invalid")?;
    Ok(Status { outgoing, incoming })
}

fn incoming_country<M: Memory>(
    memory: &M,
    countries: &Reader<'_, M>,
    array: Array,
    index: u32,
) -> Result<Country, &'static str> {
    let raw = array
        .element(index, 4)
        .and_then(|address| read::<u32>(memory, address))
        .ok_or("collaboration incoming country unreadable")?;
    countries.resolve(raw)
}

fn relation(
    memory: &impl Memory,
    base: usize,
    array: Array,
    index: u32,
    source: u32,
) -> Result<Row, &'static str> {
    let pointer = array
        .element(index, 8)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("collaboration relation pointer unreadable or null")?;
    ccountry_collaboration_data::read_row(memory, base, pointer, source)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    match walk_inner(memory, base, game_state, output) {
        Ok((len, 0)) => Outcome::Rows { len },
        Ok((len, unread_rows)) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Result<(usize, u32), &'static str> {
    let countries = Reader::new(memory, base, game_state)?;
    let mut observed = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        observed = true;
        let own_status = status(memory, base, country)?;
        output.ensure(
            len.checked_add(own_status.outgoing.count as usize)
                .ok_or("collaboration row count overflow")?,
        )?;
        for index in 0..own_status.outgoing.count {
            if len == output.len() {
                unread_rows = unread_rows
                    .checked_add(own_status.outgoing.count - index)
                    .ok_or("collaboration omitted row count overflow")?;
                break;
            }
            let row = relation(memory, base, own_status.outgoing, index, country.id)?;
            let target = countries.resolve(row.target)?;
            if target.canonical_slot == country.canonical_slot {
                return Err("collaboration relation targets its source country");
            }
            let target_status = status(memory, base, target)?;
            let mut matches = 0_u32;
            for incoming in 0..target_status.incoming.count {
                if incoming_country(memory, &countries, target_status.incoming, incoming)?
                    .canonical_slot
                    == country.canonical_slot
                {
                    matches += 1;
                }
            }
            if matches != 1 {
                return Err(
                    "collaboration outgoing relation lacks unique reciprocal incoming link",
                );
            }
            output[len] = row;
            len += 1;
        }
        if unread_rows == 0 {
            for incoming in 0..own_status.incoming.count {
                let source = incoming_country(memory, &countries, own_status.incoming, incoming)?;
                if source.canonical_slot == country.canonical_slot {
                    return Err("collaboration incoming link targets its own country");
                }
                let source_status = status(memory, base, source)?;
                let mut matches = 0_u32;
                for index in 0..source_status.outgoing.count {
                    let row = relation(memory, base, source_status.outgoing, index, source.id)?;
                    if countries.resolve(row.target)?.canonical_slot == country.canonical_slot {
                        matches += 1;
                    }
                }
                if matches != 1 {
                    return Err(
                        "collaboration incoming link lacks unique reciprocal outgoing relation",
                    );
                }
            }
        }
    }
    if !observed {
        return Err("collaboration country table has no readable countries");
    }
    output[..len].sort_unstable_by_key(|row| (row.source, row.target));
    if output[..len]
        .windows(2)
        .any(|rows| (rows[0].source, rows[0].target) == (rows[1].source, rows[1].target))
    {
        return Err("duplicate collaboration source and target");
    }
    Ok((len, unread_rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::{cpdx_array, memory::SliceMemory};

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let index = address - 0x1000;
        bytes[index..index + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], address: usize, data: u64, count: u32) {
        put(bytes, address, &data.to_le_bytes());
        put(bytes, address + cpdx_array::CAPACITY, &count.to_le_bytes());
        put(bytes, address + cpdx_array::COUNT, &count.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 4);
        array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 5);
        for (slot, pointer) in [(1, 0x3000_u64), (2, 0x5000), (3, 0x7000)] {
            put(&mut bytes, 0x2000 + slot * 8, &pointer.to_le_bytes());
            put(&mut bytes, 0x2100 + slot * 4, &(slot as u32).to_le_bytes());
            put(
                &mut bytes,
                pointer as usize,
                &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer as usize + ccountry::ID,
                &(slot as u32).to_le_bytes(),
            );
        }
        put(&mut bytes, 0x2110, &1_u32.to_le_bytes());
        put(&mut bytes, 0x3000 + ccountry::ID, &4_u32.to_le_bytes());
        for (country, status_pointer) in [(0x3000, 0x9000), (0x5000, 0x9100), (0x7000, 0x9200)] {
            put(
                &mut bytes,
                country + ccountry::COLLABORATION_STATUS,
                &(status_pointer as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                status_pointer,
                &(VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                status_pointer + OWNER,
                &(country as u64).to_le_bytes(),
            );
        }
        array(&mut bytes, 0x9000 + RELATIONS, 0x9400, 2);
        array(&mut bytes, 0x9100 + INCOMING_COUNTRIES, 0x9420, 1);
        array(&mut bytes, 0x9200 + INCOMING_COUNTRIES, 0x9430, 1);
        put(&mut bytes, 0x9400, &0x9500_u64.to_le_bytes());
        put(&mut bytes, 0x9408, &0x9600_u64.to_le_bytes());
        put(&mut bytes, 0x9420, &1_u32.to_le_bytes());
        put(&mut bytes, 0x9430, &4_u32.to_le_bytes());
        for (pointer, target, progress) in [(0x9500, 2_u32, i64::MAX), (0x9600, 3, i64::MIN)] {
            put(
                &mut bytes,
                pointer,
                &(ccountry_collaboration_data::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + ccountry_collaboration_data::TARGET_COUNTRY,
                &target.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + ccountry_collaboration_data::PROGRESS,
                &progress.to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn collaboration_preserves_native_values_and_resolves_reciprocal_aliases() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut output = vec![Row::default(); 3];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut output),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            &output[..2],
            &[
                Row {
                    source: 4,
                    target: 2,
                    progress: i64::MAX
                },
                Row {
                    source: 4,
                    target: 3,
                    progress: i64::MIN
                }
            ]
        );
        assert_eq!(
            walk_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn collaboration_rejects_missing_duplicate_and_dangling_edges() {
        for (address, data, reason) in [
            (
                0x9430,
                2_u32.to_le_bytes(),
                "collaboration outgoing relation lacks unique reciprocal incoming link",
            ),
            (
                0x9600 + ccountry_collaboration_data::TARGET_COUNTRY,
                2_u32.to_le_bytes(),
                "collaboration incoming link lacks unique reciprocal outgoing relation",
            ),
            (
                0x9500 + ccountry_collaboration_data::TARGET_COUNTRY,
                4_u32.to_le_bytes(),
                "collaboration relation targets its source country",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &data);
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0,
                    0x1000,
                    &mut vec![Row::default(); 3]
                ),
                Outcome::Unavailable { reason }
            );
        }
        let mut bytes = fixture();
        array(&mut bytes, 0x9100 + INCOMING_COUNTRIES, 0x9420, 2);
        put(&mut bytes, 0x9424, &4_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut vec![Row::default(); 3]
            ),
            Outcome::Unavailable {
                reason: "collaboration outgoing relation lacks unique reciprocal incoming link"
            }
        );
        let mut bytes = fixture();
        array(&mut bytes, 0x9100 + INCOMING_COUNTRIES, 0x9420, 2);
        put(&mut bytes, 0x9424, &3_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut vec![Row::default(); 3]
            ),
            Outcome::Unavailable {
                reason: "collaboration incoming link lacks unique reciprocal outgoing relation"
            }
        );
    }

    #[test]
    fn collaboration_faults_and_status_authentication_are_unavailable() {
        for (address, value, reason) in [
            (
                0x9200,
                0,
                "collaboration status vtable mismatch or unreadable",
            ),
            (
                0x9200 + OWNER,
                0x3000,
                "collaboration status owner mismatch or unreadable",
            ),
            (
                0x9600,
                0,
                "collaboration relation vtable mismatch or unreadable",
            ),
            (
                0x9408,
                0,
                "collaboration relation pointer unreadable or null",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &u64::to_le_bytes(value));
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0,
                    0x1000,
                    &mut vec![Row::default(); 3]
                ),
                Outcome::Unavailable { reason }
            );
        }
        let bytes = fixture();
        let end = 0x9600 + ccountry_collaboration_data::PROGRESS + 7 - 0x1000;
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..end]
                },
                0,
                0x1000,
                &mut vec![Row::default(); 3]
            ),
            Outcome::Unavailable {
                reason: "collaboration progress unreadable"
            }
        );
    }
}
