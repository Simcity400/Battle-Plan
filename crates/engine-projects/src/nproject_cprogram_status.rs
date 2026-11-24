
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
    pub country: engine_native_memory::country_identity::Country,
    pub status: usize,
    pub block: Option<[u8; BLOCK_LEN]>,
}

impl BatchedStatus {
    pub const EMPTY: Self = Self {
        country: engine_native_memory::country_identity::Country {
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
    batch: &[engine_native_memory::country_identity::Country],
    out: &mut [BatchedStatus; BATCH],
) -> (usize, Option<&'static str>) {
    let n = batch.len();
    if n == 0 {
        return (0, None);
    }
    let Some(expected) = engine_sites::at(base, VTABLE_RVA).map(|v| v as u64) else {
        return (0, Some("module address overflow"));
    };
    let Some(secondary) = engine_sites::at(base, SECONDARY_VTABLE_RVA).map(|v| v as u64) else {
        return (0, Some("module address overflow"));
    };
    let mut country_ptrs = [0_usize; BATCH];
    let mut ptr_bytes = [0_u8; BATCH * 8];
    let mut safe_count = 0;
    for (i, country) in batch.iter().enumerate() {
        if country.pointer != 0
            && country.pointer.checked_add(ccountry::PROGRAM_STATUS + 8).is_some()
        {
            country_ptrs[i] = country.pointer;
            safe_count += 1;
        }
    }
    let gathered_ptrs = safe_count == n;
    let ptrs_copied = if gathered_ptrs {
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
    let mut limit = n;
    let mut ptr_err = None;
    if gathered_ptrs {
        for i in 0..ptrs_copied {
            let Ok(p) = usize::try_from(u64::from_le_bytes(
                ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
            )) else {
                limit = i;
                ptr_err = Some("project status pointer unreadable");
                break;
            };
            if p == 0 {
                limit = i;
                ptr_err = Some("project status pointer is null");
                break;
            }
            status_ptrs[i] = p;
            if p.checked_add(BLOCK_LEN).is_some() {
                block_ptrs[block_count] = p;
                block_slots[block_count] = i;
                block_count += 1;
            }
        }
        if ptr_err.is_none() && ptrs_copied < n {
            limit = ptrs_copied;
            ptr_err = Some("project status pointer unreadable");
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
    for (i, &country) in batch[..limit].iter().enumerate() {
        let k = block_index[i];
        if k != usize::MAX {
            let slice = &blocks[k * BLOCK_LEN..(k + 1) * BLOCK_LEN];
            if u64::from_le_bytes(slice[..8].try_into().unwrap()) != expected
                || u64::from_le_bytes(slice[8..16].try_into().unwrap()) != secondary
            {
                return (i, Some("project status vtable mismatch"));
            }
            let owner = u32::from_le_bytes(slice[16..20].try_into().unwrap());
            match countries.resolve(owner) {
                Ok(resolved) if resolved.pointer == country.pointer => {}
                Ok(_) => return (i, Some("project status owner mismatch")),
                Err(err) => return (i, Some(err)),
            }
            let mut block = [0_u8; BLOCK_LEN];
            block.copy_from_slice(slice);
            out[i] = BatchedStatus {
                country,
                status: status_ptrs[i],
                block: Some(block),
            };
        } else {
            let status = if gathered_ptrs {
                match verify_status(
                    memory,
                    expected,
                    secondary,
                    country,
                    status_ptrs[i],
                    countries,
                ) {
                    Ok(status) => status,
                    Err(err) => return (i, Some(err)),
                }
            } else {
                match from_country(memory, base, country, countries) {
                    Ok(status) => status,
                    Err(err) => return (i, Some(err)),
                }
            };
            out[i] = BatchedStatus {
                country,
                status,
                block: None,
            };
        }
    }
    (limit, ptr_err)
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

fn verify_status(
    memory: &impl Memory,
    expected: u64,
    secondary: u64,
    country: engine_native_memory::country_identity::Country,
    pointer: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<usize, &'static str> {
    let header = read::<[u8; 20]>(memory, pointer).ok_or("project status header unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(header[8..16].try_into().unwrap()) != secondary
    {
        return Err("project status vtable mismatch");
    }
    let owner = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if countries.resolve(owner)?.pointer != country.pointer {
        return Err("project status owner mismatch");
    }
    Ok(pointer)
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
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("module address overflow")? as u64;
    verify_status(memory, expected, secondary, country, pointer, countries)
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

    #[test]
    fn read_batch_preserves_one_shot_pointer_fault_and_reuses_copied_pointer_on_block_fallback() {
        use std::cell::Cell;
        struct FaultMemory<'a> {
            bytes: &'a [u8],
            fault_at: usize,
            once: bool,
            faulted: Cell<bool>,
            watch_at: usize,
            watch_reads: Cell<usize>,
        }
        impl Memory for FaultMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let end = address.saturating_add(destination.len());
                if address <= self.watch_at && self.watch_at < end {
                    self.watch_reads.set(self.watch_reads.get() + 1);
                }
                if address <= self.fault_at
                    && self.fault_at < end
                    && (!self.once || !self.faulted.replace(true))
                {
                    return false;
                }
                let Some(slice) = self.bytes.get(address..end) else {
                    return false;
                };
                destination.copy_from_slice(slice);
                true
            }
        }
        const GAME: usize = 0x1000;
        const ARRAY: usize = 0x2000;
        const LINKS: usize = 0x2100;
        const COUNTRY_1: usize = 0x3000;
        const COUNTRY_2: usize = 0x5000;
        const STATUS_1: usize = 0x7000;
        const STATUS_2: usize = 0x7400;
        let mut bytes = vec![0; 0x8000];
        let put = |bytes: &mut [u8], address, value: u64| {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        let put32 = |bytes: &mut [u8], address, value: u32| {
            bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut bytes, GAME + cgame_state::COUNTRIES, ARRAY as u64);
        put(&mut bytes, GAME + cgame_state::COUNTRIES + 8, (3 << 32) | 3);
        put(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS as u64);
        put(&mut bytes, GAME + cgame_state::COUNTRY_LINKS + 8, (3 << 32) | 3);
        put(&mut bytes, ARRAY + 8, COUNTRY_1 as u64);
        put(&mut bytes, ARRAY + 16, COUNTRY_2 as u64);
        put32(&mut bytes, LINKS + 4, 1);
        put32(&mut bytes, LINKS + 8, 2);
        for (country, id, status) in [(COUNTRY_1, 1, STATUS_1), (COUNTRY_2, 2, STATUS_2)] {
            put(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            put32(&mut bytes, country + ccountry::ID, id);
            put(&mut bytes, country + ccountry::PROGRAM_STATUS, status as u64);
            put(&mut bytes, status, VTABLE_RVA.0 as u64);
            put(&mut bytes, status + SECONDARY_VTABLE, SECONDARY_VTABLE_RVA.0 as u64);
            put32(&mut bytes, status + OWNER, id);
        }
        let batch = [
            engine_native_memory::country_identity::Country {
                pointer: COUNTRY_1,
                canonical_slot: 1,
                id: 1,
            },
            engine_native_memory::country_identity::Country {
                pointer: COUNTRY_2,
                canonical_slot: 2,
                id: 2,
            },
        ];

        let ptr_fault = FaultMemory {
            bytes: &bytes,
            fault_at: COUNTRY_2 + ccountry::PROGRAM_STATUS,
            once: true,
            faulted: Cell::new(false),
            watch_at: COUNTRY_2 + ccountry::PROGRAM_STATUS,
            watch_reads: Cell::new(0),
        };
        let countries = ccountry::Reader::new(&ptr_fault, 0, GAME).unwrap();
        let mut out = [BatchedStatus::EMPTY; BATCH];
        let (ok_len, err) = read_batch(&ptr_fault, 0, &countries, &batch, &mut out);
        assert_eq!((ok_len, err), (1, Some("project status pointer unreadable")));
        assert_eq!(out[0].status, STATUS_1);
        assert_eq!(ptr_fault.watch_reads.get(), 1);

        let block_tail_fault = FaultMemory {
            bytes: &bytes,
            fault_at: STATUS_2 + BREAKTHROUGH,
            once: false,
            faulted: Cell::new(false),
            watch_at: COUNTRY_2 + ccountry::PROGRAM_STATUS,
            watch_reads: Cell::new(0),
        };
        let countries = ccountry::Reader::new(&block_tail_fault, 0, GAME).unwrap();
        let mut out = [BatchedStatus::EMPTY; BATCH];
        let (ok_len, err) = read_batch(&block_tail_fault, 0, &countries, &batch, &mut out);
        assert_eq!((ok_len, err), (2, None));
        assert_eq!(out[0].status, STATUS_1);
        assert!(out[0].block.is_some());
        assert_eq!(out[1].status, STATUS_2);
        assert!(out[1].block.is_none());
        assert_eq!(block_tail_fault.watch_reads.get(), 1);
    }
}
