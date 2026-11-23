
use engine_native_memory::{Memory, cpdx_array::Array, read};
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
