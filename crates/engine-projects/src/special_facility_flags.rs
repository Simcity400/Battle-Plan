
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::ccountry;
use crate::{cgraphical_map, cin_game_idler, nproject_cprogram, nproject_cprogram_status};

pub const FACILITY_FLAG: usize = 0x175b;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    pub flag: u8,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
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
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let graphics = province_graphics(memory, base)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let program_vtable = engine_sites::at(base, nproject_cprogram::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let (mut len, mut unread) = (0_usize, 0_u32);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let status = nproject_cprogram_status::from_country(memory, base, country, &countries)?;
            let programs = nproject_cprogram_status::programs(memory, status)?;
            for index in 0..programs.count {
                let program = programs
                    .element(index, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("program pointer unreadable or null")?;
                let header = read::<[u8; 8]>(memory, program).ok_or("program unreadable")?;
                if u64::from_le_bytes(header) != program_vtable {
                    return Err("program vtable mismatch");
                }
                let province = program
                    .checked_add(nproject_cprogram::PROVINCE_PRIMARY)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("program province unreadable")?;
                if province == 0 {
                    continue;
                }
                if province >= graphics.count {
                    return Err("program province outside the province graphics");
                }
                let node = graphics
                    .element(province, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("province graphics node unreadable or null")?;
                let flag = node
                    .checked_add(FACILITY_FLAG)
                    .and_then(|p| read::<u8>(memory, p))
                    .ok_or("province facility flag unreadable")?;
                if flag == 0 {
                    continue;
                }
                output.ensure(len.checked_add(1).ok_or("facility flag count overflow")?)?;
                if len == output.len() {
                    unread = unread
                        .checked_add(1)
                        .ok_or("facility flag count overflow")?;
                    continue;
                }
                output[len] = Row { province, flag };
                len += 1;
            }
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn province_graphics(memory: &impl Memory, base: usize) -> Result<Array, &'static str> {
    let pointer = |address: usize| {
        read::<u64>(memory, address)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
    };
    let idler = engine_sites::at(base, cin_game_idler::SINGLETON_A_RVA)
        .and_then(pointer)
        .ok_or("in-game idler unreadable or null")?;
    let vtable =
        engine_sites::at(base, cin_game_idler::VTABLE_RVA).ok_or("module address overflow")? as u64;
    if read::<u64>(memory, idler) != Some(vtable) {
        return Err("in-game idler vtable mismatch");
    }
    let map = idler
        .checked_add(cin_game_idler::MAP_DELEGATE)
        .and_then(pointer)
        .and_then(|delegate| delegate.checked_add(cin_game_idler::GRAPHICAL_MAP))
        .and_then(pointer)
        .ok_or("graphical map unreadable or null")?;
    map.checked_add(cgraphical_map::PROVINCE_GRAPHICS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("province graphics array unreadable or invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const GAME: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const COUNTRY: usize = 0x3000;
    const STATUS: usize = 0x3400;
    const PROGRAMS: usize = 0x3600;
    const PROGRAM: [usize; 2] = [0x3800, 0x3900];
    const IDLER: usize = 0x4000;
    const DELEGATE: usize = 0x4800;
    const MAP: usize = 0x5000;
    const NODES: usize = 0x5400;
    const NODE: usize = 0x6000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x10000];
        array(&mut bytes, GAME + cgame_state::COUNTRIES, COUNTRIES, 2);
        put(&mut bytes, COUNTRIES + 8, &(COUNTRY as u64).to_le_bytes());
        array(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS, 2);
        put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY + ccountry::PROGRAM_STATUS,
            &(STATUS as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS,
            &(nproject_cprogram_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + 8,
            &(nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + nproject_cprogram_status::OWNER,
            &1_u32.to_le_bytes(),
        );
        array(
            &mut bytes,
            STATUS + nproject_cprogram_status::PROGRAMS,
            PROGRAMS,
            2,
        );
        for (index, (program, province)) in PROGRAM.into_iter().zip([2_u32, 3]).enumerate() {
            put(
                &mut bytes,
                PROGRAMS + index * 8,
                &(program as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                program,
                &(nproject_cprogram::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                program + nproject_cprogram::PROVINCE_PRIMARY,
                &province.to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            IDLER,
            &(cin_game_idler::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            IDLER + cin_game_idler::MAP_DELEGATE,
            &(DELEGATE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DELEGATE + cin_game_idler::GRAPHICAL_MAP,
            &(MAP as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            MAP + cgraphical_map::PROVINCE_GRAPHICS,
            NODES,
            4,
        );
        for province in 1..4 {
            let node = NODE + province * 0x2000;
            put(
                &mut bytes,
                NODES + province * 8,
                &(node as u64).to_le_bytes(),
            );
        }
        bytes[NODE + 2 * 0x2000 + FACILITY_FLAG] = 1;
        bytes
    }

    struct Image<'a> {
        bytes: SliceMemory<'a>,
        idler: u64,
    }

    impl Memory for Image<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == cin_game_idler::SINGLETON_A_RVA.0 && destination.len() == 8 {
                destination.copy_from_slice(&self.idler.to_le_bytes());
                return true;
            }
            self.bytes.copy(address, destination)
        }
    }

    fn run_with(bytes: &[u8], idler: usize, rows: usize) -> (Outcome, Vec<Row>) {
        let memory = Image {
            bytes: SliceMemory { base: 0, bytes },
            idler: idler as u64,
        };
        let mut output = vec![Row::default(); rows];
        let outcome = walk(&memory, 0, GAME, &mut output);
        output.truncate(outcome.len());
        (outcome, output)
    }

    fn run(bytes: &[u8], rows: usize) -> (Outcome, Vec<Row>) {
        run_with(bytes, IDLER, rows)
    }

    #[test]
    fn only_flagged_program_provinces_become_rows() {
        let bytes = fixture();
        assert_eq!(
            run(&bytes, 4),
            (
                Outcome::Rows { len: 1 },
                vec![Row {
                    province: 2,
                    flag: 1
                }]
            )
        );
        assert_eq!(
            count(
                &Image {
                    bytes: SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    idler: IDLER as u64,
                },
                0,
                GAME
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        let mut outside = bytes.clone();
        put(
            &mut outside,
            PROGRAM[1] + nproject_cprogram::PROVINCE_PRIMARY,
            &9_u32.to_le_bytes(),
        );
        assert!(matches!(run(&outside, 4).0, Outcome::Unavailable { .. }));
        assert!(matches!(
            run_with(&bytes, 0, 4).0,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn growing_facility_flag_rows_preserve_mod_sized_program_occurrences() {
        let mut output = Vec::new();
        for count in [5000_usize, 12000] {
            let mut bytes = fixture();
            let list = 0x10000;
            bytes.resize(list + count * 8, 0);
            array(
                &mut bytes,
                STATUS + nproject_cprogram_status::PROGRAMS,
                list,
                count as u32,
            );
            for index in 0..count {
                put(
                    &mut bytes,
                    list + index * 8,
                    &(PROGRAM[0] as u64).to_le_bytes(),
                );
            }
            let memory = Image {
                bytes: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                idler: IDLER as u64,
            };
            assert_eq!(
                super::count(&memory, 0, GAME),
                Outcome::Truncated {
                    len: 0,
                    unread_rows: count as u32
                }
            );
            assert_eq!(
                walk(&memory, 0, GAME, &mut output),
                Outcome::Rows { len: count }
            );
            assert!(output[..count].iter().all(|row| *row
                == Row {
                    province: 2,
                    flag: 1
                }));
        }
    }
}
