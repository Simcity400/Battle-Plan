
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::ccountry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a2_34f0);
pub const COUNTRY: usize = 0x08;
pub const ARMY: usize = 0x10;
pub const ARMY_DAILY: usize = 0x18;
pub const ARMY_DAILY_TRAINING: usize = 0x20;
pub const NAVY: usize = 0x28;
pub const NAVY_DAILY: usize = 0x30;
pub const NAVY_DAILY_TRAINING: usize = 0x38;
pub const AIR: usize = 0x40;
pub const AIR_DAILY: usize = 0x48;
pub const ARMIES_FOR_TRAINING: usize = 0x50;
pub const SCALE: i64 = 32_768;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub army_raw: i64,
    pub army_daily_raw: i64,
    pub army_daily_training_raw: i64,
    pub navy_raw: i64,
    pub navy_daily_raw: i64,
    pub navy_daily_training_raw: i64,
    pub air_raw: i64,
    pub air_daily_raw: i64,
    pub armies_for_training_raw: i64,
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
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let vtable =
            engine_sites::at(base, VTABLE_RVA).ok_or("experience module address overflow")?;
        output.ensure(countries.count().saturating_sub(1) as usize)?;
        let mut len = 0_usize;
        let mut unread_rows = 0;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            if len == output.len() {
                unread_rows += 1;
                continue;
            }
            output[len] = read_country(memory, vtable, country)?;
            len += 1;
        }
        Ok(if unread_rows != 0 {
            Outcome::Truncated { len, unread_rows }
        } else {
            Outcome::Rows { len }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn read_country(
    memory: &impl Memory,
    vtable: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<Row, &'static str> {
    let status = country
        .pointer
        .checked_add(ccountry::EXPERIENCE_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("experience status pointer unreadable or null")?;
    let mut bytes = [0; ARMIES_FOR_TRAINING + 8];
    if !memory.copy(status, &mut bytes) {
        return Err("experience status scalar block unreadable");
    }
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != vtable as u64 {
        return Err("experience status vtable mismatch");
    }
    if u64::from_le_bytes(bytes[COUNTRY..COUNTRY + 8].try_into().unwrap()) != country.pointer as u64
    {
        return Err("experience status country mismatch");
    }
    let value = |offset| i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    Ok(Row {
        country: country.id,
        army_raw: value(ARMY),
        army_daily_raw: value(ARMY_DAILY),
        army_daily_training_raw: value(ARMY_DAILY_TRAINING),
        navy_raw: value(NAVY),
        navy_daily_raw: value(NAVY_DAILY),
        navy_daily_training_raw: value(NAVY_DAILY_TRAINING),
        air_raw: value(AIR),
        air_daily_raw: value(AIR_DAILY),
        armies_for_training_raw: value(ARMIES_FOR_TRAINING),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const MEMORY_BASE: usize = 0x1000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - MEMORY_BASE..address - MEMORY_BASE + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x7000];
        for (header, data) in [
            (cgame_state::COUNTRIES, 0x1800_u64),
            (cgame_state::COUNTRY_LINKS, 0x1900),
        ] {
            put(&mut bytes, MEMORY_BASE + header, &data.to_le_bytes());
            put(&mut bytes, MEMORY_BASE + header + 8, &4_u32.to_le_bytes());
            put(&mut bytes, MEMORY_BASE + header + 12, &4_u32.to_le_bytes());
        }
        for (slot, pointer, canonical) in [(1, 0x3000_u64, 1_u32), (2, 0x5000, 2), (3, 0x3000, 1)] {
            put(&mut bytes, 0x1800 + slot * 8, &pointer.to_le_bytes());
            put(&mut bytes, 0x1900 + slot * 4, &canonical.to_le_bytes());
        }
        for (country, id, status) in [(0x3000, 3_u32, 0x7000_u64), (0x5000, 2, 0x7200)] {
            put(
                &mut bytes,
                country,
                &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
            put(
                &mut bytes,
                country + ccountry::EXPERIENCE_STATUS,
                &status.to_le_bytes(),
            );
            put(
                &mut bytes,
                status as usize,
                &(VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                status as usize + COUNTRY,
                &(country as u64).to_le_bytes(),
            );
        }
        for (offset, value) in [
            (ARMY, (1_i64 << 40) + 1),
            (ARMY_DAILY, -((1_i64 << 41) + 3)),
            (ARMY_DAILY_TRAINING, (1_i64 << 46) + 13),
            (NAVY, i64::MAX),
            (NAVY_DAILY, i64::MIN),
            (NAVY_DAILY_TRAINING, -((1_i64 << 47) + 15)),
            (AIR, (1_i64 << 42) + 7),
            (AIR_DAILY, -((1_i64 << 43) + 9)),
            (ARMIES_FOR_TRAINING, (1_i64 << 44) + 11),
        ] {
            put(&mut bytes, 0x7000 + offset, &value.to_le_bytes());
        }
        bytes
    }

    fn capture(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        walk(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes,
            },
            0,
            MEMORY_BASE,
            output,
        )
    }

    fn count(bytes: &[u8]) -> Outcome {
        walk_buffer(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes,
            },
            0,
            MEMORY_BASE,
            engine_native_memory::walk::Buffer::Count,
        )
    }

    #[test]
    fn preserves_every_bit_and_native_country_alias_without_duplicate_rows() {
        let bytes = fixture();
        let mut output = vec![Row::default(); 3];
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output[0],
            Row {
                country: 3,
                army_raw: (1_i64 << 40) + 1,
                army_daily_raw: -((1_i64 << 41) + 3),
                army_daily_training_raw: (1_i64 << 46) + 13,
                navy_raw: i64::MAX,
                navy_daily_raw: i64::MIN,
                navy_daily_training_raw: -((1_i64 << 47) + 15),
                air_raw: (1_i64 << 42) + 7,
                air_daily_raw: -((1_i64 << 43) + 9),
                armies_for_training_raw: (1_i64 << 44) + 11,
            }
        );
        assert_eq!(
            output[1],
            Row {
                country: 2,
                ..Row::default()
            }
        );
    }

    #[test]
    fn the_count_probe_counts_canonical_rows_and_a_full_read_recovers() {
        let bytes = fixture();
        assert_eq!(
            count(&bytes),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        let mut output = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
    }

    #[test]
    fn null_and_wrong_status_identity_refuse_the_table() {
        for (address, value, reason) in [
            (
                0x3000 + ccountry::EXPERIENCE_STATUS,
                0,
                "experience status pointer unreadable or null",
            ),
            (0x7000, 1, "experience status vtable mismatch"),
            (
                0x7000 + COUNTRY,
                0x5000,
                "experience status country mismatch",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &u64::to_le_bytes(value));
            assert_eq!(
                capture(&bytes, &mut vec![Row::default(); 2]),
                Outcome::Unavailable { reason }
            );
        }
    }

    #[test]
    fn unreadable_channels_cannot_publish_a_partial_row_as_complete() {
        let bytes = fixture();
        let end = 0x7000 + ARMIES_FOR_TRAINING + 7 - MEMORY_BASE;
        assert_eq!(
            capture(&bytes[..end], &mut vec![Row::default(); 2]),
            Outcome::Unavailable {
                reason: "experience status scalar block unreadable",
            }
        );
    }

    #[test]
    fn malformed_country_after_capacity_is_unavailable() {
        let mut bytes = fixture();
        put(&mut bytes, 0x5000, &1_u64.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut vec![Row::default(); 1]),
            Outcome::Unavailable {
                reason: "country vtable mismatch",
            }
        );
    }

    #[test]
    fn empty_authenticated_country_census_is_empty() {
        let mut bytes = fixture();
        for slot in 1..4 {
            put(&mut bytes, 0x1800 + slot * 8, &0_u64.to_le_bytes());
        }
        assert_eq!(count(&bytes), Outcome::Rows { len: 0 });
    }
}
