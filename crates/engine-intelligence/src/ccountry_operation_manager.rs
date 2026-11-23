
use engine_native_memory::{
    Memory,
    msvc_string::Literal,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use engine_native::{ccountry, crelation::subject::Name};
use engine_native_intelligence::{coperation_instance, operation_assigned_operative};

const COUNTRY_BATCH: usize = 64;
const OP_MGR_PTR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::OPERATION_MANAGER,
    destination_offset: 0,
    len: 8,
}];
const OP_MGR_PTR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&OP_MGR_PTR_FIELDS, 8).expect("valid operation manager ptr layout");
const OP_MGR_HEAD_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: OWNER_COUNTRY,
    destination_offset: 0,
    len: 24,
}];
const OP_MGR_HEAD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&OP_MGR_HEAD_FIELDS, 24).expect("valid operation manager head layout");

pub const OWNER_COUNTRY: usize = 0x08;
#[cfg(test)]
pub const OPERATION_DATA: usize = 0x10;
#[cfg(test)]
pub const OPERATION_COUNT: usize = 0x1c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub assignments: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub parts_len: usize,
    pub required_parts: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub assignments: usize,
    pub description_bytes: usize,
    pub parts: usize,
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    let result = count(memory, base, game_state);
    let count = |outcome| match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("operation demand count overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    };
    Ok(Demand {
        rows: count(result.outcome)?,
        assignments: count(result.assignments)?,
        description_bytes: result.required_bytes,
        parts: result.required_parts,
    })
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<coperation_instance::Row>,
    assigned: &mut Vec<operation_assigned_operative::Row>,
    descriptions: &mut Vec<u8>,
    parts: &mut Vec<coperation_instance::Part>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(assigned),
        engine_native_memory::walk::Buffer::Growing(descriptions),
        engine_native_memory::walk::Buffer::Growing(parts),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, coperation_instance::Row>,
    mut assigned: engine_native_memory::walk::Buffer<'_, operation_assigned_operative::Row>,
    mut descriptions: engine_native_memory::walk::Buffer<'_, u8>,
    mut parts: engine_native_memory::walk::Buffer<'_, coperation_instance::Part>,
) -> Walk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut assigned_len = 0_usize;
        let mut assigned_unread = 0_u32;
        let mut assigned_error = None;
        let (mut bytes_len, mut required_bytes) = (0_usize, 0_usize);
        let (mut parts_len, mut required_parts) = (0_usize, 0_usize);
        let mut batch_countries = [ccountry::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; COUNTRY_BATCH];
        let mut country_ptrs = [0_usize; COUNTRY_BATCH];
        let mut mgr_ptr_bytes = [0_u8; COUNTRY_BATCH * 8];
        let mut mgr_ptrs = [0_usize; COUNTRY_BATCH];
        let mut mgr_head_bytes = [0_u8; COUNTRY_BATCH * 24];
        let mut slot = 1_u32;
        while slot < countries.count() {
            let mut n = 0_usize;
            let mut all_safe = true;
            while slot < countries.count() && n < COUNTRY_BATCH {
                let cur = slot;
                slot += 1;
                let Some(country) = countries.at(cur)? else {
                    continue;
                };
                found_country = true;
                batch_countries[n] = country;
                country_ptrs[n] = country.pointer;
                all_safe &= country
                    .pointer
                    .checked_add(ccountry::OPERATION_MANAGER + 8)
                    .is_some();
                n += 1;
            }
            if n == 0 {
                break;
            }
            let ptrs_copied = if all_safe {
                memory.copy_records(
                    RecordSources::Pointers(&country_ptrs[..n]),
                    &OP_MGR_PTR_LAYOUT,
                    &mut mgr_ptr_bytes[..n * 8],
                )
            } else {
                0
            };
            let mut head_safe = ptrs_copied == n;
            if head_safe {
                for i in 0..n {
                    let mgr = usize::try_from(u64::from_le_bytes(
                        mgr_ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                    ))
                    .ok()
                    .filter(|&p| p != 0);
                    match mgr {
                        Some(p) if p.checked_add(OWNER_COUNTRY + 24).is_some() => {
                            mgr_ptrs[i] = p;
                        }
                        _ => {
                            head_safe = false;
                            break;
                        }
                    }
                }
            }
            let heads_copied = if head_safe {
                memory.copy_records(
                    RecordSources::Pointers(&mgr_ptrs[..n]),
                    &OP_MGR_HEAD_LAYOUT,
                    &mut mgr_head_bytes[..n * 24],
                )
            } else {
                0
            };
            for i in 0..n {
                let country = batch_countries[i];
                let head: [u8; 24] = if i < heads_copied {
                    mgr_head_bytes[i * 24..(i + 1) * 24].try_into().unwrap()
                } else {
                    let manager = country
                        .pointer
                        .checked_add(ccountry::OPERATION_MANAGER)
                        .and_then(|p| read::<u64>(memory, p))
                        .and_then(|p| usize::try_from(p).ok())
                        .filter(|&p| p != 0)
                        .ok_or("country operation manager unreadable or null")?;
                    manager
                        .checked_add(OWNER_COUNTRY)
                        .and_then(|p| read::<[u8; 24]>(memory, p))
                        .ok_or("operation manager header unreadable")?
                };
                if u64::from_le_bytes(head[..8].try_into().unwrap()) != country.pointer as u64 {
                    return Err("operation manager owner mismatch");
                }
                let roster = Array::from_header(head[8..].try_into().unwrap(), 8)
                    .ok_or("operation roster unreadable or invalid")?;
                output.ensure(
                    len.checked_add(roster.count as usize)
                        .ok_or("operation row demand overflow")?,
                )?;
                for index in 0..roster.count {
                    let pointer = roster
                        .element(index, 8)
                        .and_then(|p| read::<u64>(memory, p))
                        .and_then(|p| usize::try_from(p).ok())
                        .filter(|&p| p != 0)
                        .ok_or("operation roster pointer unreadable or null")?;
                    let mut operation = coperation_instance::instance(memory, base, pointer)?;
                    if operation.owner_pointer != country.pointer {
                        return Err("operation instance owner mismatch");
                    }
                    if operation.row.target != 0 {
                        countries.resolve(operation.row.target)?;
                    }
                    operation.row.owner = country.id;
                    required_bytes = required_bytes
                        .checked_add(operation.effect_description.len)
                        .ok_or("operation description byte demand overflow")?;
                    descriptions.ensure(required_bytes)?;
                    let part_count = operation.row.part_count();
                    required_parts = required_parts
                        .checked_add(part_count)
                        .ok_or("operation part demand overflow")?;
                    parts.ensure(required_parts)?;
                    let fits = output.len() != len
                        && operation.effect_description.len <= descriptions.len() - bytes_len
                        && part_count <= parts.len() - parts_len;
                    let mut written = 0;
                    operation.parts(memory, base, |part| {
                        if fits {
                            parts[parts_len + written] = part;
                            written += 1;
                        }
                    })?;
                    if !fits {
                        unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("operation omitted count overflow")?;
                    } else {
                        Literal::copy_all(
                            memory,
                            [&operation.effect_description],
                            &mut descriptions
                                [bytes_len..bytes_len + operation.effect_description.len],
                        )
                        .map_err(|_| "operation effect description bytes unreadable")?;
                        operation.row.effect_description = Name {
                            start: bytes_len,
                            len: operation.effect_description.len,
                        };
                        bytes_len += operation.effect_description.len;
                        operation.row.parts_start = parts_len;
                        parts_len += part_count;
                        output[len] = operation.row;
                        len += 1;
                    }
                    if assigned_error.is_none() {
                        if let Err(reason) = assigned.ensure(
                            assigned_len
                                .checked_add(operation.assigned.count as usize)
                                .ok_or("operation assigned demand overflow")?,
                        ) {
                            assigned_error = Some(reason);
                            continue;
                        }
                        for ordinal in 0..operation.assigned.count {
                            if assigned_len == assigned.len() {
                                assigned_unread = assigned_unread
                                    .checked_add(operation.assigned.count - ordinal)
                                    .ok_or("operation assigned omitted count overflow")?;
                                break;
                            }
                            let Some(bytes) = operation
                                .assigned
                                .element(ordinal, operation_assigned_operative::STRIDE)
                                .and_then(|p| read::<[u8; 12]>(memory, p))
                            else {
                                assigned_error = Some("operation assigned slot unreadable");
                                break;
                            };
                            assigned[assigned_len] = operation_assigned_operative::Row {
                                operation: operation.row.id,
                                ordinal,
                                operative: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                                selection: u32::from_le_bytes(bytes[8..].try_into().unwrap()),
                            };
                            assigned_len += 1;
                        }
                    }
                }
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        output[..len].sort_unstable_by_key(|row| row.id);
        if output[..len]
            .windows(2)
            .any(|pair| pair[0].id == pair[1].id)
        {
            return Err("duplicate operation identity");
        }
        let outcome = if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        };
        let assignments = if let Some(reason) = assigned_error {
            Outcome::Unavailable { reason }
        } else {
            assigned[..assigned_len].sort_unstable_by_key(|row| (row.operation, row.ordinal));
            if assigned[..assigned_len].windows(2).any(|pair| {
                (pair[0].operation, pair[0].ordinal) == (pair[1].operation, pair[1].ordinal)
            }) {
                Outcome::Unavailable {
                    reason: "duplicate operation assigned-slot identity",
                }
            } else if assigned_unread == 0 {
                Outcome::Rows { len: assigned_len }
            } else {
                Outcome::Truncated {
                    len: assigned_len,
                    unread_rows: assigned_unread,
                }
            }
        };
        Ok(Walk {
            outcome,
            assignments,
            bytes_len,
            required_bytes,
            parts_len,
            required_parts,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        assignments: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
        parts_len: 0,
        required_parts: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_intelligence::coperation;
    use engine_native_memory::memory::SliceMemory;
    use coperation_instance::{Equipment, EquipmentTotal, Part, Requirement};
    use engine_native_parts::cequipment_type;
    use engine_native::cequipment_variant::{self, pool as cequipment_variant_pool};

    const BASE: usize = 0x140_000_000;
    const GAME: usize = 0x1000;
    const COUNTRY: usize = 0x2000;
    const MANAGER: usize = 0x4000;
    const INSTANCE: usize = 0x4200;
    const DEFINITION: usize = 0x5000;
    const ROSTER: usize = 0x7200;
    const ASSIGNED: usize = 0x7400;
    const PHASES: usize = 0x7600;
    const PHASE_DEFINITIONS: usize = 0x7700;
    const EQUIPMENT_TYPE: usize = 0x5400;
    const VARIANT: usize = 0x5c00;
    const REQUIREMENTS: usize = 0x5d00;
    const POOL_ITEMS: usize = 0x5e00;
    const POOL_TOTALS: usize = 0x5f00;
    const ARCHETYPE: usize = 0x6000;
    const ID: u64 = 0xfedc_ba98_7654_3210;
    const VARIANT_HANDLE: u64 = (77 << 32) | cequipment_variant::REFERENCE_TYPE as u64;

    fn parts() -> Vec<Part> {
        vec![Part::default(); 1 << 14]
    }

    fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        u64_at(bytes, at, data as u64);
        u32_at(bytes, at + 8, count);
        u32_at(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, GAME + cgame_state::COUNTRIES, 0x7000, 2);
        array(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, 0x7100, 2);
        u64_at(&mut bytes, 0x7008, COUNTRY as u64);
        u32_at(&mut bytes, 0x7104, 1);
        u64_at(&mut bytes, COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        u32_at(&mut bytes, COUNTRY + ccountry::ID, 1);
        u64_at(
            &mut bytes,
            COUNTRY + ccountry::OPERATION_MANAGER,
            MANAGER as u64,
        );
        u64_at(&mut bytes, MANAGER + OWNER_COUNTRY, COUNTRY as u64);
        array(&mut bytes, MANAGER + OPERATION_DATA, ROSTER, 1);
        u64_at(&mut bytes, ROSTER, INSTANCE as u64);
        u64_at(
            &mut bytes,
            INSTANCE,
            (BASE + coperation_instance::VTABLE_RVA.0) as u64,
        );
        u64_at(&mut bytes, INSTANCE + coperation_instance::HANDLE, ID);
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::PREPARATION_CURRENT,
            (i64::MIN + 17) as u64,
        );
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::PREPARATION_REQUIRED,
            i64::MAX as u64,
        );
        bytes[INSTANCE + coperation_instance::AUTO_REPEAT] = 1;
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::EFFECT_DESCRIPTION + 24,
            15,
        );
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::DEFINITION,
            DEFINITION as u64,
        );
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::OWNER_COUNTRY,
            COUNTRY as u64,
        );
        u32_at(
            &mut bytes,
            INSTANCE + coperation_instance::TARGET_COUNTRY_ID,
            1,
        );
        u32_at(
            &mut bytes,
            INSTANCE + coperation_instance::START_HOUR,
            (-9_i32) as u32,
        );
        u32_at(
            &mut bytes,
            INSTANCE + coperation_instance::DURATION_DAYS,
            (-2_i32) as u32,
        );
        array(
            &mut bytes,
            INSTANCE + coperation_instance::PHASE_DATA,
            PHASES,
            2,
        );
        for (index, token) in [0x11_u32, 0x22].into_iter().enumerate() {
            let definition = PHASE_DEFINITIONS + index * 0x20;
            u64_at(&mut bytes, PHASES + index * 8, definition as u64);
            u32_at(
                &mut bytes,
                definition + coperation_instance::PHASE_TOKEN,
                token,
            );
        }
        u64_at(
            &mut bytes,
            INSTANCE + coperation_instance::EQUIPMENT_POOL,
            (BASE + cequipment_variant_pool::VTABLE_RVA.0) as u64,
        );
        array(
            &mut bytes,
            INSTANCE + coperation_instance::ASSIGNED_OPERATIVE_DATA,
            ASSIGNED,
            2,
        );
        u64_at(
            &mut bytes,
            ASSIGNED + operation_assigned_operative::STRIDE,
            0x1234_5678_abcd_9876,
        );
        u32_at(
            &mut bytes,
            ASSIGNED + operation_assigned_operative::STRIDE + 8,
            u32::MAX,
        );
        u64_at(
            &mut bytes,
            DEFINITION,
            (BASE + coperation::VTABLE_RVA.0) as u64,
        );
        u32_at(&mut bytes, DEFINITION + coperation::TOKEN_ID, 0xfabc_1234);
        bytes[DEFINITION + coperation::PREPARATION_TIME..DEFINITION + 0x48].fill(0xaa);
        bytes
    }

    #[test]
    fn growing_operation_rows_assignments_and_description_capture_the_first_header() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut assigned = Vec::new();
        let mut descriptions = Vec::new();
        let mut grown_parts = Vec::new();
        for count in [2_usize, 5_000] {
            const ASSIGNMENTS: usize = 0x10000;
            const DESCRIPTION: usize = 0x80000;
            bytes.resize(DESCRIPTION + count + 1, 0);
            array(
                &mut bytes,
                INSTANCE + coperation_instance::ASSIGNED_OPERATIVE_DATA,
                ASSIGNMENTS,
                count as u32,
            );
            for index in 0..count {
                u64_at(
                    &mut bytes,
                    ASSIGNMENTS + index * operation_assigned_operative::STRIDE,
                    index as u64,
                );
                u32_at(
                    &mut bytes,
                    ASSIGNMENTS + index * operation_assigned_operative::STRIDE + 8,
                    index as u32,
                );
            }
            let header = INSTANCE + coperation_instance::EFFECT_DESCRIPTION;
            u64_at(&mut bytes, header, DESCRIPTION as u64);
            u64_at(&mut bytes, header + 16, count as u64);
            u64_at(&mut bytes, header + 24, count.max(16) as u64);
            bytes[DESCRIPTION..DESCRIPTION + count].fill(0xfe);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut expected_rows = vec![coperation_instance::Row::default(); 1];
            let mut expected_assigned = vec![operation_assigned_operative::Row::default(); count];
            let mut expected_description = vec![0; count];
            let expected = walk(
                &memory,
                BASE,
                GAME,
                &mut expected_rows,
                &mut expected_assigned,
                &mut expected_description,
                &mut parts(),
            );
            assert_eq!(expected.outcome, Outcome::Rows { len: 1 });
            assert_eq!(expected.assignments, Outcome::Rows { len: count });
            let actual = walk(
                &memory,
                BASE,
                GAME,
                &mut rows,
                &mut assigned,
                &mut descriptions,
                &mut grown_parts,
            );
            assert_eq!(actual, expected);
            assert_eq!(&rows[..1], &expected_rows);
            assert_eq!(&assigned[..count], expected_assigned.as_slice());
            assert_eq!(&descriptions[..count], expected_description.as_slice());
        }
    }

    #[test]
    fn native_values_definition_token_and_empty_assigned_slots_are_preserved() {
        let bytes = fixture();
        let mut rows = vec![coperation_instance::Row::default(); 2];
        let mut assignments = vec![operation_assigned_operative::Row::default(); 3];
        let result = walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            BASE,
            GAME,
            &mut rows,
            &mut assignments,
            &mut Vec::new(),
            &mut parts(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.assignments, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].id, ID);
        assert_eq!(rows[0].owner, 1);
        assert_eq!(rows[0].target, 1);
        assert_eq!(rows[0].preparation_current, i64::MIN + 17);
        assert_eq!(rows[0].preparation_required, i64::MAX);
        assert_eq!(rows[0].start_hour, -9);
        assert_eq!(rows[0].duration_days, -2);
        assert_eq!(rows[0].definition_token, 0xfabc_1234);
        assert_eq!(assignments[0].operative, 0);
        assert_eq!(assignments[1].operative, 0x1234_5678_abcd_9876);
        assert_eq!(assignments[1].selection, u32::MAX);
    }

    #[test]
    fn readable_empty_roster_differs_from_missing_or_invalid_manager() {
        let mut bytes = fixture();
        array(&mut bytes, MANAGER + OPERATION_DATA, 0, 0);
        let mut rows = vec![];
        let mut assignments = vec![];
        let run = |bytes: &[u8], rows: &mut Vec<_>, assignments: &mut Vec<_>| {
            walk(
                &SliceMemory { base: 0, bytes },
                BASE,
                GAME,
                rows,
                assignments,
                &mut Vec::new(),
                &mut parts(),
            )
        };
        assert_eq!(
            run(&bytes, &mut rows, &mut assignments).outcome,
            Outcome::Rows { len: 0 }
        );
        u32_at(&mut bytes, MANAGER + OPERATION_COUNT, u32::MAX);
        assert!(matches!(
            run(&bytes, &mut rows, &mut assignments).outcome,
            Outcome::Unavailable { .. }
        ));
        u64_at(&mut bytes, COUNTRY + ccountry::OPERATION_MANAGER, 0);
        assert!(matches!(
            run(&bytes, &mut rows, &mut assignments).outcome,
            Outcome::Unavailable {
                reason: "country operation manager unreadable or null"
            }
        ));
    }

    #[test]
    fn stored_outcome_bytes_and_flags_have_exact_native_arena_demand() {
        let mut bytes = fixture();
        bytes[INSTANCE + coperation_instance::TERMINAL] = 1;
        bytes[INSTANCE + coperation_instance::BONUS] = 1;
        bytes[INSTANCE + coperation_instance::DETECTED] = 1;
        let description = [0xfe, b'\r', b'"', b'\n', 0, b'x'];
        let header = INSTANCE + coperation_instance::EFFECT_DESCRIPTION;
        bytes[header..header + description.len()].copy_from_slice(&description);
        u64_at(&mut bytes, header + 16, description.len() as u64);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            demand(&memory, BASE, GAME),
            Ok(Demand {
                rows: 1,
                assignments: 2,
                description_bytes: description.len(),
                parts: 2,
            })
        );
        let mut rows = vec![coperation_instance::Row::default(); 1];
        let mut assigned = vec![operation_assigned_operative::Row::default(); 2];
        let mut descriptions = vec![0; 6];
        let result = walk(
            &memory,
            BASE,
            GAME,
            &mut rows,
            &mut assigned,
            &mut descriptions,
            &mut parts(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!((result.bytes_len, result.required_bytes), (6, 6));
        assert_eq!(
            (rows[0].terminal, rows[0].bonus, rows[0].detected),
            (1, 1, 1)
        );
        assert_eq!(rows[0].effect_description, Name { start: 0, len: 6 });
        assert_eq!(descriptions, description);
    }

    #[test]
    fn heap_descriptions_have_no_chosen_cap_and_faults_withhold_the_table() {
        let mut bytes = fixture();
        let payload = 0x8000;
        let description = vec![0xfe; 40_000];
        bytes.resize(payload + description.len() + 1, 0);
        bytes[payload..payload + description.len()].copy_from_slice(&description);
        let header = INSTANCE + coperation_instance::EFFECT_DESCRIPTION;
        u64_at(&mut bytes, header, payload as u64);
        u64_at(&mut bytes, header + 16, description.len() as u64);
        u64_at(&mut bytes, header + 24, description.len() as u64);
        let mut rows = vec![coperation_instance::Row::default(); 1];
        let mut assigned = vec![operation_assigned_operative::Row::default(); 2];
        let mut descriptions = vec![0; description.len()];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            demand(&memory, BASE, GAME).unwrap().description_bytes,
            40_000
        );
        let result = walk(
            &memory,
            BASE,
            GAME,
            &mut rows,
            &mut assigned,
            &mut descriptions,
            &mut parts(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(descriptions, description);

        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                address != 0x8000
                    && SliceMemory {
                        base: 0,
                        bytes: self.0,
                    }
                    .copy(address, destination)
            }
        }
        let failed = walk(
            &Fault(&bytes),
            BASE,
            GAME,
            &mut rows,
            &mut assigned,
            &mut descriptions,
            &mut parts(),
        );
        assert_eq!(
            failed.outcome,
            Outcome::Unavailable {
                reason: "operation effect description bytes unreadable"
            }
        );
        assert_eq!(failed.bytes_len, 0);
        bytes[payload + description.len()] = 1;
        assert_eq!(
            demand(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                GAME
            ),
            Err("operation effect description terminator unreadable or invalid")
        );
        u64_at(&mut bytes, header + 16, 1_u64 << 63);
        u64_at(&mut bytes, header + 24, 1_u64 << 63);
        assert_eq!(
            demand(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                GAME
            ),
            Err("operation effect description exceeds native string maximum")
        );
    }

    #[test]
    fn invalid_outcome_flags_are_refused_without_normalization() {
        for field in [coperation_instance::BONUS, coperation_instance::DETECTED] {
            let mut bytes = fixture();
            bytes[INSTANCE + field] = 2;
            assert_eq!(
                demand(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    BASE,
                    GAME
                ),
                Err("operation boolean flag invalid")
            );
        }
    }

    #[test]
    fn assigned_payload_fault_does_not_erase_readable_operation_scalars() {
        struct FaultMemory<'a>(&'a [u8]);
        impl Memory for FaultMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == ASSIGNED {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, destination)
            }
        }
        let bytes = fixture();
        let mut rows = vec![coperation_instance::Row::default(); 2];
        let mut assignments = vec![operation_assigned_operative::Row::default(); 3];
        let result = walk(
            &FaultMemory(&bytes),
            BASE,
            GAME,
            &mut rows,
            &mut assignments,
            &mut Vec::new(),
            &mut parts(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            result.assignments,
            Outcome::Unavailable {
                reason: "operation assigned slot unreadable"
            }
        );
    }

    #[test]
    fn duplicated_identity_and_wrong_owner_refuse_the_primary_table() {
        let mut bytes = fixture();
        array(&mut bytes, MANAGER + OPERATION_DATA, ROSTER, 2);
        u64_at(&mut bytes, ROSTER + 8, INSTANCE as u64);
        let mut rows = vec![coperation_instance::Row::default(); 2];
        let mut assignments = vec![operation_assigned_operative::Row::default(); 4];
        let result = walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            BASE,
            GAME,
            &mut rows,
            &mut assignments,
            &mut Vec::new(),
            &mut parts(),
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "duplicate operation identity"
            }
        );
        array(&mut bytes, MANAGER + OPERATION_DATA, ROSTER, 1);
        u64_at(&mut bytes, INSTANCE + coperation_instance::OWNER_COUNTRY, 0);
        let result = walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            BASE,
            GAME,
            &mut rows,
            &mut assignments,
            &mut Vec::new(),
            &mut parts(),
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "operation instance owner mismatch"
            }
        );
    }

    fn prepared() -> Vec<u8> {
        let mut bytes = fixture();
        let type_vtable = (BASE + cequipment_type::VTABLE_RVA.0) as u64;
        u64_at(&mut bytes, EQUIPMENT_TYPE, type_vtable);
        u64_at(
            &mut bytes,
            EQUIPMENT_TYPE + cequipment_type::ARCHETYPE,
            ARCHETYPE as u64,
        );
        u64_at(&mut bytes, ARCHETYPE, type_vtable);
        u32_at(&mut bytes, ARCHETYPE + cequipment_type::TOKEN, 0x77);
        u64_at(
            &mut bytes,
            VARIANT,
            (BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64,
        );
        u64_at(&mut bytes, VARIANT + 8, VARIANT_HANDLE);
        u64_at(
            &mut bytes,
            VARIANT + cequipment_variant::DEFINITION,
            EQUIPMENT_TYPE as u64,
        );
        array(
            &mut bytes,
            INSTANCE + coperation_instance::REQUIREMENT_DATA,
            REQUIREMENTS,
            2,
        );
        let factory = REQUIREMENTS;
        u64_at(
            &mut bytes,
            factory + coperation_instance::REQUIREMENT_AMOUNT,
            250_000,
        );
        u32_at(
            &mut bytes,
            factory + coperation_instance::REQUIREMENT_DAYS,
            3,
        );
        bytes[factory + coperation_instance::REQUIREMENT_USES_TIME] = 1;
        let equipment = REQUIREMENTS + coperation_instance::REQUIREMENT_STRIDE;
        u64_at(
            &mut bytes,
            equipment + coperation_instance::REQUIREMENT_AMOUNT,
            1_000_000,
        );
        u64_at(
            &mut bytes,
            equipment + coperation_instance::REQUIREMENT_EQUIPMENT,
            EQUIPMENT_TYPE as u64,
        );
        let pool = INSTANCE + coperation_instance::EQUIPMENT_POOL;
        array(
            &mut bytes,
            pool + cequipment_variant_pool::ITEMS,
            POOL_ITEMS,
            2,
        );
        for (index, amount) in [500_000_i64, 0].into_iter().enumerate() {
            let entry = POOL_ITEMS + index * engine_native_parts::equipment_entry::STRIDE;
            u64_at(&mut bytes, entry, VARIANT as u64);
            u64_at(&mut bytes, entry + 8, amount as u64);
        }
        array(
            &mut bytes,
            pool + cequipment_variant_pool::ALLOCATIONS,
            POOL_TOTALS,
            1,
        );
        u64_at(&mut bytes, POOL_TOTALS, ARCHETYPE as u64);
        u64_at(
            &mut bytes,
            POOL_TOTALS + engine_native_parts::allocation_equipment_row::AMOUNT,
            500_000,
        );
        bytes[pool + cequipment_variant_pool::ALLOW_ZERO] = 1;
        bytes
    }

    fn walk_parts(bytes: &[u8], parts: &mut Vec<Part>) -> (Walk, coperation_instance::Row) {
        let mut rows = vec![coperation_instance::Row::default(); 1];
        let result = walk(
            &SliceMemory { base: 0, bytes },
            BASE,
            GAME,
            &mut rows,
            &mut vec![operation_assigned_operative::Row::default(); 2],
            &mut Vec::new(),
            parts,
        );
        (result, rows[0])
    }

    #[test]
    fn phases_requirements_and_the_equipment_pool_follow_native_order() {
        let bytes = prepared();
        let mut arena = vec![Part::default(); 7];
        let (result, row) = walk_parts(&bytes, &mut arena);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!((result.parts_len, result.required_parts), (7, 7));
        assert_eq!(
            (
                row.parts_start,
                row.phase_count,
                row.requirement_count,
                row.equipment_count,
                row.equipment_total_count,
                row.equipment_allow_zero
            ),
            (0, 2, 2, 2, 1, 1)
        );
        assert_eq!(
            arena,
            [
                Part::Phase(0x11),
                Part::Phase(0x22),
                Part::Requirement(Requirement {
                    amount: 250_000,
                    days: 3,
                    archetype: 0,
                    equipment: 0,
                    uses_time: 1,
                }),
                Part::Requirement(Requirement {
                    amount: 1_000_000,
                    days: 0,
                    archetype: 0x77,
                    equipment: 1,
                    uses_time: 0,
                }),
                Part::Equipment(Equipment {
                    variant: VARIANT_HANDLE,
                    archetype: 0x77,
                    amount: 500_000,
                }),
                Part::Equipment(Equipment {
                    variant: VARIANT_HANDLE,
                    archetype: 0x77,
                    amount: 0,
                }),
                Part::EquipmentTotal(EquipmentTotal {
                    archetype: 0x77,
                    amount: 500_000,
                }),
            ]
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(demand(&memory, BASE, GAME).unwrap().parts, 7);
    }

    #[test]
    fn a_missing_phase_definition_withholds_the_table() {
        let mut bytes = prepared();
        u64_at(&mut bytes, PHASES + 8, 0);
        assert_eq!(
            walk_parts(&bytes, &mut Vec::new()).0.outcome,
            Outcome::Unavailable {
                reason: "operation phase definition unreadable or null"
            }
        );
    }

    #[test]
    fn part_refusals_are_named() {
        let check = |change: &dyn Fn(&mut Vec<u8>), reason: &'static str| {
            let mut bytes = prepared();
            change(&mut bytes);
            assert_eq!(
                walk_parts(&bytes, &mut parts()).0.outcome,
                Outcome::Unavailable { reason },
                "{reason}"
            );
        };
        check(
            &|bytes| u64_at(bytes, INSTANCE + coperation_instance::EQUIPMENT_POOL, 0),
            "equipment pool vtable mismatch",
        );
        check(
            &|bytes| u64_at(bytes, EQUIPMENT_TYPE, 0),
            "operation equipment type vtable mismatch",
        );
        check(
            &|bytes| u64_at(bytes, EQUIPMENT_TYPE + cequipment_type::ARCHETYPE, 0),
            "operation equipment archetype null",
        );
        check(
            &|bytes| u64_at(bytes, VARIANT + 8, 77 << 32),
            "variant vtable or reference type mismatch",
        );
        check(
            &|bytes| u64_at(bytes, VARIANT + cequipment_variant::DEFINITION, 0),
            "operation equipment variant definition unreadable or null",
        );
        let mut bytes = prepared();
        u64_at(
            &mut bytes,
            EQUIPMENT_TYPE,
            (BASE + cequipment_type::NULL_VTABLE_RVA.0) as u64,
        );
        assert_eq!(
            walk_parts(&bytes, &mut parts()).0.outcome,
            Outcome::Rows { len: 1 }
        );
        check(
            &|bytes| {
                u32_at(
                    bytes,
                    INSTANCE + coperation_instance::REQUIREMENT_DATA + 12,
                    u32::MAX,
                )
            },
            "operation requirements unreadable or invalid",
        );
        check(
            &|bytes| u32_at(bytes, INSTANCE + coperation_instance::PHASE_DATA + 8, 1),
            "operation phases unreadable or invalid",
        );
    }
}
