
use crate::{
    Memory, ccountry, cdiplomacy_status,
    csubject_relation::{self, Name, NameWalk, Reader},
    memory::{RecordField, RecordLayout, RecordSources, packed_fields},
    read_slice,
    walk::Outcome,
};
use engine_sites::Rva;

const COUNTRY_BATCH: usize = 64;
const DIPLO_PTR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::DIPLOMACY_STATUS,
    destination_offset: 0,
    len: 8,
}];
const DIPLO_PTR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DIPLO_PTR_FIELDS, 8).expect("valid diplomacy pointer layout");
const DIPLO_AUTONOMY_FIELDS: [RecordField; 3] = packed_fields([
    (0, 8),
    (cdiplomacy_status::COUNTRY, 8),
    (cdiplomacy_status::CURRENT_AUTONOMY_STATUS, 8),
]);
const DIPLO_AUTONOMY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DIPLO_AUTONOMY_FIELDS, 24).expect("valid diplomacy autonomy layout");

pub const VTABLE_RVA: Rva = Rva(0x0286_1030);
pub const SCORE: usize = 0x08;
pub const STATE: usize = 0x28;
pub const PREVIOUS_STATE: usize = 0x30;
pub const LOWER_SCORE: usize = 0x38;
pub const NEXT_STATE: usize = 0x40;
pub const UPPER_SCORE: usize = 0x48;
pub const SUBJECT_COUNTRY: usize = 0x50;
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
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Count, crate::walk::Buffer::Count)
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut names: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        output.ensure(reader.countries.count() as usize)?;
        let (mut len, mut bytes_len, mut required_bytes) = (0, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        let diplo_vtable = engine_sites::at(base, cdiplomacy_status::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let max_diplo_end = cdiplomacy_status::COUNTRY
            .max(cdiplomacy_status::CURRENT_AUTONOMY_STATUS)
            + 8;
        let mut batch_countries = [ccountry::Country {
            id: 0,
            canonical_slot: 0,
            pointer: 0,
        }; COUNTRY_BATCH];
        let mut country_ptrs = [0_usize; COUNTRY_BATCH];
        let mut diplo_ptr_bytes = [0_u8; COUNTRY_BATCH * 8];
        let mut diplo_ptrs = [0_usize; COUNTRY_BATCH];
        let mut diplo_records = [0_u8; COUNTRY_BATCH * 24];
        let mut slot = 1_u32;
        while slot < reader.countries.count() {
            let mut n = 0_usize;
            let mut all_safe = true;
            while slot < reader.countries.count() && n < COUNTRY_BATCH {
                let cur = slot;
                slot += 1;
                let Some(country) = reader.countries.at(cur)? else {
                    continue;
                };
                found_country = true;
                batch_countries[n] = country;
                country_ptrs[n] = country.pointer;
                all_safe &= country
                    .pointer
                    .checked_add(ccountry::DIPLOMACY_STATUS + 8)
                    .is_some();
                n += 1;
            }
            if n == 0 {
                break;
            }
            let ptrs_copied = if all_safe {
                memory.copy_records(
                    RecordSources::Pointers(&country_ptrs[..n]),
                    &DIPLO_PTR_LAYOUT,
                    &mut diplo_ptr_bytes[..n * 8],
                )
            } else {
                0
            };
            let mut diplo_safe = ptrs_copied == n;
            if diplo_safe {
                for i in 0..n {
                    let diplo = usize::try_from(u64::from_le_bytes(
                        diplo_ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                    ))
                    .ok()
                    .filter(|&p| p != 0);
                    match diplo {
                        Some(p) if p.checked_add(max_diplo_end).is_some() => {
                            diplo_ptrs[i] = p;
                        }
                        _ => {
                            diplo_safe = false;
                            break;
                        }
                    }
                }
            }
            let diplos_copied = if diplo_safe {
                memory.copy_records(
                    RecordSources::Pointers(&diplo_ptrs[..n]),
                    &DIPLO_AUTONOMY_LAYOUT,
                    &mut diplo_records[..n * 24],
                ) / DIPLO_AUTONOMY_FIELDS.len()
            } else {
                0
            };
            for i in 0..n {
                let country = batch_countries[i];
                let pointer = if i < diplos_copied {
                    let rec = &diplo_records[i * 24..(i + 1) * 24];
                    if u64::from_le_bytes(rec[..8].try_into().unwrap()) != diplo_vtable {
                        return Err("diplomacy vtable mismatch");
                    }
                    if u64::from_le_bytes(rec[8..16].try_into().unwrap()) != country.pointer as u64 {
                        return Err("diplomacy country mismatch");
                    }
                    usize::try_from(u64::from_le_bytes(rec[16..24].try_into().unwrap()))
                        .map_err(|_| "subject pointer overflow")?
                } else {
                    let diplomacy = reader.diplomacy(country)?;
                    reader.pointer(diplomacy, cdiplomacy_status::CURRENT_AUTONOMY_STATUS)?
                };
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
    use crate::{
        autonomy_state_definition, ccountry, cgame_state, memory::SliceMemory, msvc_string,
    };

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
