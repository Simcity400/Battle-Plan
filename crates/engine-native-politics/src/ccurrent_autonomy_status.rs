pub use engine_native_parts::ccurrent_autonomy_status_layout::{VTABLE_RVA, SUBJECT_COUNTRY, STATE};

use engine_native_memory::{Memory, read_slice, walk::Outcome};
use crate::cdiplomacy_status;
use engine_native::crelation::subject::{
    self as csubject_relation,
    Name,
    NameWalk,
    Reader,
};

pub const SCORE: usize = 0x08;
pub const PREVIOUS_STATE: usize = 0x30;
pub const LOWER_SCORE: usize = 0x38;
pub const NEXT_STATE: usize = 0x40;
pub const UPPER_SCORE: usize = 0x48;
pub const LAST_MODIFIED_HOUR: usize = 0x60;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x006c_2010;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub subject: u32,
    pub score: i64,
    pub lower_score: i64,
    pub upper_score: i64,
    pub last_modified_bits: u32,
    pub state: Name,
    pub previous: Option<Name>,
    pub next: Option<Name>,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        output.ensure(reader.countries.count() as usize)?;
        let (mut len, mut bytes_len, mut required_bytes) = (0, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..reader.countries.count() {
            let Some(country) = reader.countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let diplomacy = reader.diplomacy(country)?;
            let pointer = reader.pointer(diplomacy, cdiplomacy_status::CURRENT_AUTONOMY_STATUS)?;
            if pointer == 0 {
                continue;
            }
            reader.vtable(pointer, VTABLE_RVA, "current autonomy vtable mismatch")?;
            if reader.pointer(pointer, SUBJECT_COUNTRY)? != country.pointer {
                return Err("current autonomy subject country mismatch");
            }
            let states = [
                reader.pointer(pointer, STATE)?,
                reader.pointer(pointer, PREVIOUS_STATE)?,
                reader.pointer(pointer, NEXT_STATE)?,
            ];
            let mut sources = [None; 3];
            let mut row_bytes = 0_usize;
            for (index, definition) in states.into_iter().enumerate() {
                if index != 0 && definition == 0 {
                    continue;
                }
                let source = csubject_relation::definition_name(memory, definition)?;
                row_bytes = row_bytes
                    .checked_add(source.1)
                    .ok_or("current autonomy name byte count overflow")?;
                sources[index] = Some(source);
            }
            required_bytes = required_bytes
                .checked_add(row_bytes)
                .ok_or("current autonomy name byte count overflow")?;
            let mut row = Row {
                subject: country.id,
                score: reader.field(pointer, SCORE)?,
                lower_score: reader.field(pointer, LOWER_SCORE)?,
                upper_score: reader.field(pointer, UPPER_SCORE)?,
                last_modified_bits: reader.field(pointer, LAST_MODIFIED_HOUR)?,
                ..Row::default()
            };
            names.ensure(required_bytes)?;
            if len == output.len() || row_bytes > names.len() - bytes_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("current autonomy row count overflow")?;
                continue;
            }
            let mut fields = [None; 3];
            for (index, source) in sources.into_iter().enumerate() {
                if let Some((source, name_len)) = source {
                    if !read_slice(memory, source, &mut names[bytes_len..bytes_len + name_len]) {
                        return Err("current autonomy name bytes unreadable");
                    }
                    fields[index] = Some(Name {
                        start: bytes_len,
                        len: name_len,
                    });
                    bytes_len += name_len;
                }
            }
            row.state = fields[0].ok_or("current autonomy state definition is null")?;
            row.previous = fields[1];
            row.next = fields[2];
            output[len] = row;
            len += 1;
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(csubject_relation::unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::autonomy_state_definition;
    use engine_native::{ccountry, cgame_state};
    use engine_native_memory::{memory::SliceMemory, msvc_string};

    const GS: usize = 0x1000;
    const COUNTRY: usize = 0x4000;
    const DIPLOMACY: usize = 0x6000;
    const STATUS: usize = 0x7000;
    const DEFINITIONS: [usize; 3] = [0x8000, 0x8100, 0x8200];
    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xa000];
        p64(&mut bytes, GS + cgame_state::COUNTRIES, 0x3000);
        p32(&mut bytes, GS + cgame_state::COUNTRIES + 8, 3);
        p32(&mut bytes, GS + cgame_state::COUNTRIES + 12, 3);
        p64(&mut bytes, 0x3000, u64::MAX);
        p64(&mut bytes, 0x3008, COUNTRY as u64);
        p64(&mut bytes, 0x3010, COUNTRY as u64);
        p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 3);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 3);
        p32(&mut bytes, 0x3104, 1);
        p32(&mut bytes, 0x3108, 1);
        p64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, COUNTRY + ccountry::ID, 2);
        p64(
            &mut bytes,
            COUNTRY + ccountry::DIPLOMACY_STATUS,
            DIPLOMACY as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY,
            cdiplomacy_status::VTABLE_RVA.0 as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY + cdiplomacy_status::COUNTRY,
            COUNTRY as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY + cdiplomacy_status::CURRENT_AUTONOMY_STATUS,
            STATUS as u64,
        );
        p64(&mut bytes, STATUS, VTABLE_RVA.0 as u64);
        p64(&mut bytes, STATUS + SUBJECT_COUNTRY, COUNTRY as u64);
        p64(&mut bytes, STATUS + SCORE, i64::MIN as u64);
        p64(&mut bytes, STATUS + LOWER_SCORE, i64::MAX as u64);
        p64(&mut bytes, STATUS + UPPER_SCORE, 0x1_0000_0001);
        p32(&mut bytes, STATUS + LAST_MODIFIED_HOUR, u32::MAX);
        for (index, offset) in [STATE, PREVIOUS_STATE, NEXT_STATE].into_iter().enumerate() {
            p64(&mut bytes, STATUS + offset, DEFINITIONS[index] as u64);
            let name = DEFINITIONS[index] + autonomy_state_definition::NAME;
            bytes[name] = [b'a', 255, b'c'][index];
            p64(&mut bytes, name + msvc_string::LENGTH, 1);
            p64(&mut bytes, name + msvc_string::CAPACITY, 15);
        }
        bytes
    }
    fn capture(bytes: &[u8], output: &mut Vec<Row>, names: &mut Vec<u8>) -> NameWalk {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, output, names)
    }

    #[test]
    fn native_scores_nullable_identities_and_alias_are_preserved_without_ratio_projection() {
        let mut bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut names = vec![0; 3];
        assert_eq!(
            capture(&bytes, &mut rows, &mut names),
            NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 3,
                required_bytes: 3
            }
        );
        assert_eq!(
            rows[0],
            Row {
                subject: 2,
                score: i64::MIN,
                lower_score: i64::MAX,
                upper_score: 0x1_0000_0001,
                last_modified_bits: u32::MAX,
                state: Name { start: 0, len: 1 },
                previous: Some(Name { start: 1, len: 1 }),
                next: Some(Name { start: 2, len: 1 })
            }
        );
        assert_eq!(names, [b'a', 255, b'c']);
        p64(&mut bytes, STATUS + PREVIOUS_STATE, 0);
        p64(&mut bytes, STATUS + NEXT_STATE, 0);
        assert_eq!(
            capture(&bytes, &mut rows, &mut names),
            NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 1,
                required_bytes: 1
            }
        );
        assert_eq!(rows[0].previous, None);
        assert_eq!(rows[0].next, None);
    }

    #[test]
    fn the_count_probe_counts_complete_rows_and_all_three_native_names() {
        let bytes = fixture();
        assert_eq!(
            count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
            NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                bytes_len: 0,
                required_bytes: 3
            }
        );
    }

    #[test]
    fn null_status_is_absence_but_bad_identity_and_missing_current_state_are_unavailable() {
        let mut bytes = fixture();
        p64(
            &mut bytes,
            DIPLOMACY + cdiplomacy_status::CURRENT_AUTONOMY_STATUS,
            0,
        );
        assert_eq!(
            capture(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
        for corruption in 0..4 {
            let mut bytes = fixture();
            match corruption {
                0 => p64(&mut bytes, STATUS, 1),
                1 => p64(&mut bytes, STATUS + SUBJECT_COUNTRY, 0),
                2 => p64(&mut bytes, STATUS + STATE, 0),
                _ => p64(&mut bytes, STATUS + NEXT_STATE, 0x12345678),
            }
            assert!(matches!(
                capture(&bytes, &mut vec![Row::default(); 1], &mut vec![0; 50]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }
}
