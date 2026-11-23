
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::ccountry;
use crate::{nproject_cprogram as program, nproject_cprogram_status as status};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub id: u32,
    pub owner: u32,
    pub province: u32,
    pub active_project: Option<u32>,
    pub project_definition: Option<u32>,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("raid facility field unreadable")
}

fn row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    country: u32,
    ordinal: u32,
) -> Result<Row, &'static str> {
    if pointer == 0 {
        return Err("raid facility pointer is null");
    }
    let header: [u8; 0x20] = field(memory, pointer, 0)?;
    let expected = engine_sites::at(base, program::VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("raid facility vtable mismatch");
    }
    let word = |at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    if word(program::TYPE) != program::REFERENCE_TYPE || word(program::ID) == 0 {
        return Err("raid facility identity invalid");
    }
    let active: [u8; 5] = field(memory, pointer, program::ACTIVE_PROJECT_HANDLE)?;
    if active[4] > 1 {
        return Err("raid facility project presence invalid");
    }
    Ok(Row {
        country,
        ordinal,
        id: word(program::ID),
        owner: word(program::OWNER_PRIMARY),
        province: word(program::PROVINCE_PRIMARY),
        active_project: (active[4] != 0)
            .then(|| u32::from_le_bytes(active[..4].try_into().unwrap())),
        project_definition: None,
    })
}

pub fn count(memory: &impl Memory, base: usize, game: usize) -> Outcome {
    walk_buffer(memory, base, game, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game, engine_native_memory::walk::Buffer::Growing(output))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game)?;
        let mut len = 0usize;
        let mut found = false;
        let mut batch_countries = [engine_native_memory::country_identity::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; status::BATCH];
        let mut statuses = [status::BatchedStatus::EMPTY; status::BATCH];
        let mut slot = 1_u32;
        while slot < countries.count() {
            let mut n = 0_usize;
            while slot < countries.count() && n < status::BATCH {
                let cur = slot;
                slot += 1;
                let Some(country) = countries.at(cur)? else {
                    continue;
                };
                found = true;
                batch_countries[n] = country;
                n += 1;
            }
            if n == 0 {
                break;
            }
            status::read_batch(memory, base, &countries, &batch_countries[..n], &mut statuses)?;
            for entry in &statuses[..n] {
                let country = entry.country;
                let programs = match &entry.block {
                    Some(block) => status::programs_from_block(block)?,
                    None => status::programs(memory, entry.status)?,
                };
                output.ensure(
                    len.checked_add(programs.count as usize)
                        .ok_or("raid facility row count overflow")?,
                )?;
                for ordinal in 0..programs.count {
                    if let Some(out) = output.get_mut(len) {
                        let pointer = programs
                            .element(ordinal, 8)
                            .and_then(|at| read::<u64>(memory, at))
                            .and_then(|pointer| usize::try_from(pointer).ok())
                            .ok_or("raid facility pointer unreadable")?;
                        *out = row(memory, base, pointer, country.id, ordinal)?;
                        let owner = countries.resolve(out.owner)?;
                        if let Some(token) = out.active_project {
                            let owner_status = status::from_country(memory, base, owner, &countries)?;
                            if let Some((definition, project_owner)) =
                                crate::nproject_cproject_pool::active_definition(
                                    memory,
                                    base,
                                    owner_status,
                                    token,
                                )?
                            {
                                if countries.resolve(project_owner)?.pointer != owner.pointer {
                                    return Err("active project owner differs from pool owner");
                                }
                                out.project_definition = Some(definition);
                            }
                        }
                    }
                    len = len.checked_add(1).ok_or("raid facility count overflow")?;
                }
            }
        }
        if !found {
            return Err("raid facility country roster contains no countries");
        }
        Ok(if len > output.len() {
            Outcome::Truncated {
                len: output.len(),
                unread_rows: u32::try_from(len - output.len())
                    .map_err(|_| "raid facility count exceeds u32")?,
            }
        } else {
            Outcome::Rows { len }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn fixture(count: usize) -> (Vec<u8>, usize) {
        use engine_native::cgame_state;
        let array = 0x2000;
        let links = array + (count + 1) * 8;
        let start = links + (count + 1) * 4;
        let mut bytes = vec![0u8; start + count * 0x1200];
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        for (offset, pointer) in [
            (cgame_state::COUNTRIES, array),
            (cgame_state::COUNTRY_LINKS, links),
        ] {
            put(&mut bytes, 0x1000 + offset, pointer as u64);
            put(
                &mut bytes,
                0x1000 + offset + 8,
                ((count as u64 + 1) << 32) | (count as u64 + 1),
            );
        }
        for slot in 1..=count {
            let country = start + (slot - 1) * 0x1200;
            let status = country + 0x1000;
            let list = country + 0x1080;
            let program = country + 0x1100;
            put(&mut bytes, array + slot * 8, country as u64);
            bytes[links + slot * 4..links + slot * 4 + 4]
                .copy_from_slice(&(slot as u32).to_le_bytes());
            put(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            put(&mut bytes, country + ccountry::ID, slot as u64);
            put(
                &mut bytes,
                country + ccountry::PROGRAM_STATUS,
                status as u64,
            );
            put(&mut bytes, status, status::VTABLE_RVA.0 as u64);
            put(
                &mut bytes,
                status + 8,
                status::SECONDARY_VTABLE_RVA.0 as u64,
            );
            put(&mut bytes, status + 0x10, slot as u64);
            put(&mut bytes, status + 0x20, list as u64);
            put(&mut bytes, status + 0x28, (2u64 << 32) | 2);
            put(&mut bytes, list, program as u64);
            put(&mut bytes, list + 8, program as u64);
            put(&mut bytes, program, program::VTABLE_RVA.0 as u64);
            put(
                &mut bytes,
                program + 8,
                (slot as u64) << 32 | program::REFERENCE_TYPE as u64,
            );
            put(
                &mut bytes,
                program + 0x18,
                (slot as u64) << 32 | slot as u64,
            );
        }
        (bytes, start)
    }

    #[test]
    fn first_and_later_native_rosters_grow_without_omitting_occurrences() {
        let mut rows = Vec::new();
        for countries in [3, 7] {
            let (bytes, _) = fixture(countries);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(
                count(&memory, 0, 0x1000),
                Outcome::Truncated {
                    len: 0,
                    unread_rows: countries as u32 * 2
                }
            );
            assert_eq!(
                walk(&memory, 0, 0x1000, &mut rows),
                Outcome::Rows { len: countries * 2 }
            );
            assert_eq!(rows[countries * 2 - 1].country, countries as u32);
            assert_eq!(rows[countries * 2 - 1].ordinal, 1);
        }
    }

    #[test]
    fn roster_order_capacity_and_faults() {
        let (mut bytes, start) = fixture(3);
        let mut rows = vec![Row::default(); 6];
        let run =
            |bytes: &[u8], rows: &mut Vec<Row>| walk(&SliceMemory { base: 0, bytes }, 0, 0x1000, rows);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 6 });
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.country, (index / 2 + 1) as u32);
            assert_eq!(row.ordinal, (index % 2) as u32);
            assert_eq!(row.id, row.country);
        }
        bytes[start + 0x1100] ^= 1;
        assert_eq!(
            run(&bytes, &mut rows),
            Outcome::Unavailable {
                reason: "raid facility vtable mismatch"
            }
        );
        assert!(matches!(
            run(&bytes[..0x2100], &mut rows),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn optional_project_preserves_zero_and_ignores_inactive_storage() {
        let mut bytes = vec![0u8; 0x56];
        bytes[..8].copy_from_slice(&(program::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..12].copy_from_slice(&program::REFERENCE_TYPE.to_le_bytes());
        bytes[12..16].copy_from_slice(&7u32.to_le_bytes());
        let read_row = |bytes: &[u8]| {
            row(
                &SliceMemory {
                    base: 0x1000,
                    bytes,
                },
                0,
                0x1000,
                4,
                2,
            )
        };
        assert_eq!(read_row(&bytes).unwrap().active_project, None);
        bytes[0x54] = 1;
        assert_eq!(read_row(&bytes).unwrap().active_project, Some(0));
        bytes[0x50..0x54].copy_from_slice(&991u32.to_le_bytes());
        assert_eq!(read_row(&bytes).unwrap().active_project, Some(991));
        bytes[0x54] = 2;
        assert!(read_row(&bytes).is_err());
        assert!(read_row(&bytes[..0x54]).is_err());
    }
    #[test]
    #[ignore = "synthetic acquisition cost; release, affinity 0xF"]
    fn acquisition_cost() {
        let countries = std::env::var("BATTLEPLAN_RAID_SCORE_COUNTRIES").or_else(|_| std::env::var("HOI4_RAID_SCORE_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1500);
        let (bytes, _) = fixture(countries);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); countries * 2];
        let mut samples = Vec::new();
        for _ in 0..40 {
            let start = std::time::Instant::now();
            let result = walk(&memory, 0, 0x1000, &mut rows);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(matches!(result, Outcome::Rows { .. }));
            std::hint::black_box(&rows);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "raid_facilities countries={countries} median_ms={} max_ms={} output_bytes={}",
            samples[20],
            samples[39],
            std::mem::size_of_val(rows.as_slice())
        );
    }
}
