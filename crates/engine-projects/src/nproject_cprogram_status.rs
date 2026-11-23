
use engine_native_memory::{
    Memory, cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};
use engine_native::ccountry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029e_e0d8);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029e_e0f8);
pub const SECONDARY_VTABLE: usize = 0x08;
pub const SIZE: usize = 0xa0;
pub const OWNER: usize = 0x10;
pub const PROJECT_POOL: usize = 0x18;
pub const PROGRAMS: usize = 0x20;
pub const BREAKTHROUGH: usize = 0x50;
pub const BATCH: usize = 64;
pub const BLOCK_LEN: usize = 0x68;

const PTR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::PROGRAM_STATUS,
    destination_offset: 0,
    len: 8,
}];
const PTR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PTR_FIELDS, 8).expect("valid program status pointer layout");
const BLOCK_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: BLOCK_LEN,
}];
const BLOCK_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BLOCK_FIELDS, BLOCK_LEN).expect("valid program status block layout");

#[derive(Clone, Copy)]
pub struct BatchedStatus {
    pub country: ccountry::Country,
    pub status: usize,
    pub block: Option<[u8; BLOCK_LEN]>,
}

impl BatchedStatus {
    pub const EMPTY: Self = Self {
        country: ccountry::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        },
        status: 0,
        block: None,
    };
}

pub fn read_batch(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    batch: &[ccountry::Country],
    out: &mut [BatchedStatus; BATCH],
) -> Result<usize, &'static str> {
    let n = batch.len();
    if n == 0 {
        return Ok(0);
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("module address overflow")? as u64;
    let mut country_ptrs = [0_usize; BATCH];
    let mut ptr_bytes = [0_u8; BATCH * 8];
    let mut safe_count = 0;
    for (i, country) in batch.iter().enumerate() {
        if country.pointer.checked_add(ccountry::PROGRAM_STATUS + 8).is_some() {
            country_ptrs[i] = country.pointer;
            safe_count += 1;
        }
    }
    let ptrs_copied = if safe_count == n {
        memory.copy_records(
            RecordSources::Pointers(&country_ptrs[..n]),
            &PTR_LAYOUT,
            &mut ptr_bytes[..n * 8],
        )
    } else {
        0
    };
    let mut status_ptrs = [0_usize; BATCH];
    let mut block_ptrs = [0_usize; BATCH];
    let mut block_slots = [0_usize; BATCH];
    let mut block_count = 0_usize;
    for (i, &_country) in batch.iter().enumerate() {
        let status = if i < ptrs_copied {
            let p = usize::try_from(u64::from_le_bytes(
                ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
            ))
            .map_err(|_| "project status pointer unreadable")?;
            if p == 0 {
                return Err("project status pointer is null");
            }
            p
        } else {
            0
        };
        status_ptrs[i] = status;
        if status != 0 && status.checked_add(BLOCK_LEN).is_some() {
            block_ptrs[block_count] = status;
            block_slots[block_count] = i;
            block_count += 1;
        }
    }
    let mut blocks = [0_u8; BATCH * BLOCK_LEN];
    let blocks_copied = if block_count > 0 {
        memory.copy_records(
            RecordSources::Pointers(&block_ptrs[..block_count]),
            &BLOCK_LAYOUT,
            &mut blocks[..block_count * BLOCK_LEN],
        )
    } else {
        0
    };
    let mut block_index = [usize::MAX; BATCH];
    for k in 0..blocks_copied {
        block_index[block_slots[k]] = k;
    }
    for (i, &country) in batch.iter().enumerate() {
        let k = block_index[i];
        if k != usize::MAX {
            let slice = &blocks[k * BLOCK_LEN..(k + 1) * BLOCK_LEN];
            if u64::from_le_bytes(slice[..8].try_into().unwrap()) != expected
                || u64::from_le_bytes(slice[8..16].try_into().unwrap()) != secondary
            {
                return Err("project status vtable mismatch");
            }
            let owner = u32::from_le_bytes(slice[16..20].try_into().unwrap());
            if countries.resolve(owner)?.pointer != country.pointer {
                return Err("project status owner mismatch");
            }
            let mut block = [0_u8; BLOCK_LEN];
            block.copy_from_slice(slice);
            out[i] = BatchedStatus {
                country,
                status: status_ptrs[i],
                block: Some(block),
            };
        } else {
            let status = from_country(memory, base, country, countries)?;
            out[i] = BatchedStatus {
                country,
                status,
                block: None,
            };
        }
    }
    Ok(n)
}

pub fn programs_from_block(block: &[u8; BLOCK_LEN]) -> Result<Array, &'static str> {
    Array::from_header(block[PROGRAMS..PROGRAMS + 16].try_into().unwrap(), 8)
        .ok_or("program array unreadable or invalid")
}

pub fn pool_from_block(block: &[u8; BLOCK_LEN]) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(
        block[PROJECT_POOL..PROJECT_POOL + 8].try_into().unwrap(),
    ))
    .ok()
    .filter(|&p| p != 0)
    .ok_or("project pool pointer unreadable or null")
}

pub fn from_country(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<usize, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::PROGRAM_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("project status pointer unreadable")?;
    if pointer == 0 {
        return Err("project status pointer is null");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("module address overflow")?;
    let header = read::<[u8; 20]>(memory, pointer).ok_or("project status header unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
        || u64::from_le_bytes(header[8..16].try_into().unwrap()) != secondary as u64
    {
        return Err("project status vtable mismatch");
    }
    let owner = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if countries.resolve(owner)?.pointer != country.pointer {
        return Err("project status owner mismatch");
    }
    Ok(pointer)
}

pub fn programs(memory: &impl Memory, status: usize) -> Result<Array, &'static str> {
    status
        .checked_add(PROGRAMS)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("program array unreadable or invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::{memory::SliceMemory, walk::Outcome};
    use crate::{
        nproject_cbreakthrough_progress,
        nproject_cprogram,
        nproject_cproject,
        nproject_sproject_history,
    };

    #[test]
    fn missing_country_or_status_root_never_becomes_four_empty_tables() {
        const GAME: usize = 0x1000;
        const ARRAY: usize = 0x2000;
        const LINKS: usize = 0x2100;
        const COUNTRY: usize = 0x3000;
        let mut bytes = vec![0; 0x5000];
        let put = |bytes: &mut [u8], address, value: u64| {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut bytes, GAME + cgame_state::COUNTRIES, ARRAY as u64);
        put(&mut bytes, GAME + cgame_state::COUNTRIES + 8, (2 << 32) | 2);
        put(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS as u64);
        put(
            &mut bytes,
            GAME + cgame_state::COUNTRY_LINKS + 8,
            (2 << 32) | 2,
        );
        put(&mut bytes, LINKS, 1 << 32);
        put(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put(&mut bytes, COUNTRY + ccountry::ID, 1);
        for country_pointer in [0, COUNTRY as u64] {
            put(&mut bytes, ARRAY + 8, country_pointer);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let outcomes = [
                nproject_cprogram::walk(&memory, 0, GAME, &mut Vec::new(), &mut Vec::new(), &mut Vec::new()).outcome,
                nproject_cproject::walk(&memory, 0, GAME, &mut Vec::new()),
                nproject_sproject_history::walk(&memory, 0, GAME, &mut Vec::new()),
                nproject_cbreakthrough_progress::walk(&memory, 0, GAME, &mut Vec::new()),
            ];
            assert!(
                outcomes
                    .iter()
                    .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
            );
            if country_pointer != 0 {
                assert!(outcomes.iter().all(|outcome| *outcome
                    == Outcome::Unavailable {
                        reason: "project status pointer is null",
                    }));
            }
        }
    }
}
