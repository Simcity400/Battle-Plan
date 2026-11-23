
use engine_native_memory::{Memory, msvc_string, read, read_slice, walk::Outcome};
use engine_native::ccountry;
use crate::nproject_cprogram_status;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029e_e080);
pub const REFERENCE_TYPE: u32 = 83;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const OWNER_PRIMARY: usize = 0x18;
pub const PROVINCE_PRIMARY: usize = 0x1c;
pub const BUILDING: usize = 0x20;
pub const ACTIVE_PROJECT_HANDLE: usize = 0x50;
pub const ACTIVE_FLAG: usize = 0x54;
pub const SCIENTIST_REFERENCE_TYPE: usize = 0x58;
pub const OWNER_COPY: usize = 0x170;
pub const PROVINCE_COPY: usize = 0x174;
pub const UNREAD_PROTOTYPE_REWARDS: usize = 0x178;
pub const SUPPORTING_SCIENTISTS: usize = 0xb0;
pub const ASSIGNMENT_STRIDE: usize = 0x30;
pub const ASSIGNMENT_TIMER: usize = 0x10;
pub const ASSIGNMENT_TIMED: usize = 0x28;
pub const ASSIGNMENT_BYTES: usize = 17;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub scientist: u64,
    pub target: i32,
    pub current: i32,
    pub timed: u8,
}

pub fn assignments(
    memory: &impl Memory,
    program: usize,
    primary: u64,
    mut emit: impl FnMut(Assignment) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let at = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("program assignment address overflow")
    };
    let read_timer = |pointer: usize, scientist| -> Result<Assignment, &'static str> {
        if scientist == 0 {
            return Ok(Assignment::default());
        }
        let timer = read::<[u8; 8]>(memory, at(pointer, ASSIGNMENT_TIMER)?)
            .ok_or("program scientist timer unreadable")?;
        let timed = read::<u8>(memory, at(pointer, ASSIGNMENT_TIMED)?)
            .ok_or("program scientist timer flag unreadable")?;
        Ok(Assignment {
            scientist,
            target: i32::from_le_bytes(timer[..4].try_into().unwrap()),
            current: i32::from_le_bytes(timer[4..].try_into().unwrap()),
            timed,
        })
    };
    emit(read_timer(at(program, SCIENTIST_REFERENCE_TYPE)?, primary)?)?;
    let supports = engine_native_memory::cpdx_array::Array::read(
        memory,
        at(program, SUPPORTING_SCIENTISTS)?,
        ASSIGNMENT_STRIDE,
    )
    .ok_or("supporting scientist collection invalid or unreadable")?;
    for ordinal in 0..supports.count {
        let pointer = supports
            .element(ordinal, ASSIGNMENT_STRIDE)
            .ok_or("supporting scientist address overflow")?;
        let scientist =
            read::<u64>(memory, pointer).ok_or("supporting scientist reference unreadable")?;
        emit(read_timer(pointer, scientist)?)?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub owner: u32,
    pub province: u32,
    pub active_project: Option<u32>,
    pub scientist: u64,
    pub unread_rewards: i32,
    pub building_start: usize,
    pub building_len: usize,
    pub assignments_start: usize,
    pub assignments_count: u32,
    pub assignments_known: bool,
    pub details_start: usize,
    pub details_count: u32,
    pub details_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub required_candidates: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    candidates: &mut Vec<usize>,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(candidates),
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut candidates: engine_native_memory::walk::Buffer<'_, usize>,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    let mut result = NameWalk {
        outcome: Outcome::Rows { len: 0 },
        bytes_len: 0,
        required_bytes: 0,
        required_candidates: 0,
    };
    let read_result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        let mut batch_countries = [ccountry::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; nproject_cprogram_status::BATCH];
        let mut statuses = [nproject_cprogram_status::BatchedStatus::EMPTY;
            nproject_cprogram_status::BATCH];
        let mut slot = 1_u32;
        while slot < countries.count() {
            let mut n = 0_usize;
            while slot < countries.count() && n < nproject_cprogram_status::BATCH {
                let cur = slot;
                slot += 1;
                let Some(country) = countries.at(cur)? else {
                    continue;
                };
                found_country = true;
                batch_countries[n] = country;
                n += 1;
            }
            if n == 0 {
                break;
            }
            nproject_cprogram_status::read_batch(
                memory,
                base,
                &countries,
                &batch_countries[..n],
                &mut statuses,
            )?;
            for entry in &statuses[..n] {
                let programs = match &entry.block {
                    Some(block) => nproject_cprogram_status::programs_from_block(block)?,
                    None => nproject_cprogram_status::programs(memory, entry.status)?,
                };
                candidates.ensure(
                    result
                        .required_candidates
                        .checked_add(programs.count as usize)
                        .ok_or("program candidate count overflow")?,
                )?;
                for index in 0..programs.count {
                    if result.required_candidates < candidates.len() {
                        let pointer = programs
                            .element(index, 8)
                            .and_then(|p| read::<u64>(memory, p))
                            .and_then(|p| usize::try_from(p).ok())
                            .filter(|&p| p != 0)
                            .ok_or("program pointer unreadable or null")?;
                        candidates[result.required_candidates] = pointer;
                    }
                    result.required_candidates = result
                        .required_candidates
                        .checked_add(1)
                        .ok_or("program candidate count overflow")?;
                }
            }
        }
        if !found_country {
            return Err("program country roster contains no countries");
        }
        if result.required_candidates > candidates.len() {
            result.outcome = Outcome::Unavailable {
                reason: "program pointer scratch capacity insufficient",
            };
            return Ok(());
        }
        let candidates = &mut candidates[..result.required_candidates];
        candidates.sort_unstable();
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut previous = None;
        for &pointer in candidates.iter() {
            if previous == Some(pointer) {
                continue;
            }
            previous = Some(pointer);
            let (mut row, source) = read_row(memory, base, pointer)?;
            let owner = countries.resolve(row.owner)?;
            result.required_bytes = result
                .required_bytes
                .checked_add(row.building_len)
                .ok_or("program building byte count overflow")?;
            output.ensure(len.checked_add(1).ok_or("program row count overflow")?)?;
            names.ensure(result.required_bytes)?;
            if len == output.len() || row.building_len > names.len() - result.bytes_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("program row count overflow")?;
                continue;
            }
            row.building_start = result.bytes_len;
            let end = result.bytes_len + row.building_len;
            if !read_slice(memory, source, &mut names[result.bytes_len..end]) {
                return Err("program building bytes unreadable");
            }
            result.bytes_len = end;
            row.assignments_start = end;
            let mut assignment_bytes = 0usize;
            let mut assignments_fit = true;
            let assignment_read = assignments(memory, pointer, row.scientist, |assignment| {
                assignment_bytes = assignment_bytes
                    .checked_add(ASSIGNMENT_BYTES)
                    .ok_or("program assignment demand overflow")?;
                let end = result
                    .bytes_len
                    .checked_add(ASSIGNMENT_BYTES)
                    .ok_or("program assignment arena overflow")?;
                if !names.ensure(end)? {
                    assignments_fit = false;
                    return Ok(());
                }
                if !assignments_fit {
                    return Ok(());
                }
                let bytes = &mut names[result.bytes_len..end];
                bytes[..8].copy_from_slice(&assignment.scientist.to_le_bytes());
                bytes[8..12].copy_from_slice(&assignment.target.to_le_bytes());
                bytes[12..16].copy_from_slice(&assignment.current.to_le_bytes());
                bytes[16] = assignment.timed;
                result.bytes_len = end;
                row.assignments_count = row
                    .assignments_count
                    .checked_add(1)
                    .ok_or("program assignment count overflow")?;
                Ok(())
            });
            row.assignments_known = assignment_read.is_ok() && assignments_fit;
            if !row.assignments_known {
                result.bytes_len = row.assignments_start;
                row.assignments_count = 0;
            }
            result.required_bytes = result
                .required_bytes
                .checked_add(assignment_bytes)
                .ok_or("program assignment byte count overflow")?;
            row.details_start = result.bytes_len;
            if row.assignments_known {
                let mut detail_bytes = 0usize;
                let mut fit = true;
                let arena = std::cell::RefCell::new(&mut names);
                let mut detail_count = 0u32;
                let assignment_count = row.assignments_count as usize;
                let assignment_start = row.assignments_start;
                let acquired = crate::program_details::observe(
                    memory,
                    base,
                    game_state,
                    pointer,
                    &row,
                    || {
                        let status = nproject_cprogram_status::from_country(
                            memory, base, owner, &countries,
                        )?;
                        Ok((owner.pointer, status))
                    },
                    assignment_count,
                    |index| {
                        let start = assignment_start + index * ASSIGNMENT_BYTES;
                        u64::from_le_bytes(arena.borrow()[start..start + 8].try_into().unwrap())
                    },
                    |operand| {
                        let mut names = arena.borrow_mut();
                        detail_bytes = detail_bytes
                            .checked_add(crate::program_details::BYTES)
                            .ok_or("program detail demand overflow")?;
                        let end = result
                            .bytes_len
                            .checked_add(crate::program_details::BYTES)
                            .ok_or("program detail arena overflow")?;
                        if !names.ensure(end)? {
                            fit = false;
                        }
                        if fit {
                            operand.encode(&mut names[result.bytes_len..end]);
                            result.bytes_len = end;
                            detail_count = detail_count
                                .checked_add(1)
                                .ok_or("program detail count overflow")?;
                        }
                        Ok(())
                    },
                );
                row.details_count = detail_count;
                row.details_known = acquired.is_ok() && fit;
                result.required_bytes = result
                    .required_bytes
                    .checked_add(detail_bytes)
                    .ok_or("program detail byte count overflow")?;
                if !row.details_known {
                    result.bytes_len = row.details_start;
                    row.details_count = 0;
                }
            }
            output[len] = row;
            len += 1;
        }
        output[..len].sort_unstable_by_key(|row| row.id);
        if output[..len]
            .windows(2)
            .any(|rows| rows[0].id == rows[1].id)
        {
            return Err("different program objects share an id");
        }
        result.outcome = if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        };
        Ok(())
    })();
    if let Err(reason) = read_result {
        result.outcome = Outcome::Unavailable { reason };
        result.bytes_len = 0;
    }
    result
}

fn read_row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(Row, usize), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let header = read::<[u8; 16]>(memory, pointer).ok_or("program identity unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("program vtable mismatch");
    }
    if u32::from_le_bytes(header[8..12].try_into().unwrap()) != REFERENCE_TYPE {
        return Err("program reference type mismatch");
    }
    let id = u32::from_le_bytes(header[12..16].try_into().unwrap());
    if id == 0 {
        return Err("program id is zero");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("program address overflow")
    };
    let mut primary = [0; 8];
    let mut copies = [0; 12];
    let mut scientist = [0; 8];
    let mut active = [0; 1];
    let mut building = [0; 32];
    if memory.copy_scattered(
        [
            at(OWNER_PRIMARY)?,
            at(OWNER_COPY)?,
            at(SCIENTIST_REFERENCE_TYPE)?,
            at(ACTIVE_FLAG)?,
            at(BUILDING)?,
        ],
        &mut [
            &mut primary,
            &mut copies,
            &mut scientist,
            &mut active,
            &mut building,
        ],
    ) != 5
    {
        return Err("program fields unreadable");
    }
    if primary != copies[..8] {
        return Err("program owner or province copies mismatch");
    }
    let owner = u32::from_le_bytes(primary[..4].try_into().unwrap());
    let province = u32::from_le_bytes(primary[4..8].try_into().unwrap());
    if active[0] > 1 {
        return Err("program active presence invalid");
    }
    let active_project = if active[0] == 0 {
        None
    } else {
        Some(
            read::<u32>(memory, at(ACTIVE_PROJECT_HANDLE)?)
                .ok_or("program active project unreadable")?,
        )
    };
    let name_len = u64::from_le_bytes(building[16..24].try_into().unwrap());
    let name_capacity = u64::from_le_bytes(building[24..32].try_into().unwrap());
    if name_capacity < msvc_string::INLINE_CAPACITY || name_len > name_capacity {
        return Err("program building string header invalid");
    }
    let building_len = usize::try_from(name_len).map_err(|_| "program building length overflow")?;
    let source = if name_capacity == msvc_string::INLINE_CAPACITY {
        at(BUILDING)?
    } else {
        usize::try_from(u64::from_le_bytes(building[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("program building pointer invalid")?
    };
    if read::<u8>(
        memory,
        source
            .checked_add(building_len)
            .ok_or("program building address overflow")?,
    ) != Some(0)
    {
        return Err("program building terminator unreadable or invalid");
    }
    Ok((
        Row {
            id,
            owner,
            province,
            active_project,
            scientist: u64::from_le_bytes(scientist),
            unread_rewards: i32::from_le_bytes(copies[8..12].try_into().unwrap()),
            building_start: 0,
            building_len,
            assignments_start: 0,
            assignments_count: 0,
            assignments_known: false,
            details_start: 0,
            details_count: 0,
            details_known: false,
        },
        source,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn gap_research_grants_supporting_assignments_preserve_occurrences_timers_and_null_omission() {
        const BASE: usize = 0x1000;
        const DATA: usize = 0x200;
        const COUNT: usize = 3;
        let mut bytes = vec![0; DATA + COUNT * ASSIGNMENT_STRIDE];
        bytes[SUPPORTING_SCIENTISTS..SUPPORTING_SCIENTISTS + 8]
            .copy_from_slice(&((BASE + DATA) as u64).to_le_bytes());
        bytes[SUPPORTING_SCIENTISTS + 8..SUPPORTING_SCIENTISTS + 12]
            .copy_from_slice(&(COUNT as u32).to_le_bytes());
        bytes[SUPPORTING_SCIENTISTS + 12..SUPPORTING_SCIENTISTS + 16]
            .copy_from_slice(&(COUNT as i32).to_le_bytes());
        for index in 0..COUNT {
            let at = DATA + index * ASSIGNMENT_STRIDE;
            bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            bytes[at + ASSIGNMENT_TIMER..at + ASSIGNMENT_TIMER + 4]
                .copy_from_slice(&i32::MIN.to_le_bytes());
            bytes[at + ASSIGNMENT_TIMER + 4..at + ASSIGNMENT_TIMER + 8]
                .copy_from_slice(&i32::MAX.to_le_bytes());
            bytes[at + ASSIGNMENT_TIMED] = 253;
        }
        let mut values = Vec::new();
        assignments(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            BASE,
            0,
            |assignment| {
                values.push(assignment);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(values.len(), COUNT + 1);
        assert_eq!(values[0], Assignment::default());
        assert_eq!(
            values[COUNT],
            Assignment {
                scientist: u64::MAX,
                target: i32::MIN,
                current: i32::MAX,
                timed: 253
            }
        );
        bytes.truncate(DATA + COUNT * ASSIGNMENT_STRIDE - (ASSIGNMENT_STRIDE - ASSIGNMENT_TIMED));
        assert_eq!(
            assignments(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                BASE,
                0,
                |_| Ok(())
            ),
            Err("program scientist timer flag unreadable")
        );
    }

    struct AbsentPayload<'a>(SliceMemory<'a>);
    impl Memory for AbsentPayload<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let forbidden = self.0.base + ACTIVE_PROJECT_HANDLE;
            if address < forbidden + 4 && address.saturating_add(destination.len()) > forbidden {
                return false;
            }
            self.0.copy(address, destination)
        }
    }

    fn program() -> [u8; 0x188] {
        let mut bytes = [0; 0x188];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[TYPE..TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        bytes[ID..ID + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[OWNER_PRIMARY..OWNER_PRIMARY + 4].copy_from_slice(&439_u32.to_le_bytes());
        bytes[PROVINCE_PRIMARY..PROVINCE_PRIMARY + 4].copy_from_slice(&13413_u32.to_le_bytes());
        let owner_province: [u8; 8] = bytes[OWNER_PRIMARY..OWNER_PRIMARY + 8].try_into().unwrap();
        bytes[OWNER_COPY..OWNER_COPY + 8].copy_from_slice(&owner_province);
        bytes[UNREAD_PROTOTYPE_REWARDS..UNREAD_PROTOTYPE_REWARDS + 4]
            .copy_from_slice(&i32::MAX.to_le_bytes());
        bytes[BUILDING..BUILDING + 3].copy_from_slice(&[b'A', 0xff, b'B']);
        bytes[BUILDING + 16..BUILDING + 24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[BUILDING + 24..BUILDING + 32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[SCIENTIST_REFERENCE_TYPE..SCIENTIST_REFERENCE_TYPE + 8]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        bytes
    }

    #[test]
    fn absent_active_project_never_reads_payload_and_preserves_native_fields() {
        let bytes = program();
        let memory = AbsentPayload(SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        });
        let (row, source) = read_row(&memory, 0, 0x1000).unwrap();
        assert_eq!(row.id, u32::MAX);
        assert_eq!(row.owner, 439);
        assert_eq!(row.province, 13413);
        assert_eq!(row.active_project, None);
        assert_eq!(row.scientist, u64::MAX);
        assert_eq!(row.unread_rewards, i32::MAX);
        assert_eq!(row.building_len, 3);
        let mut name = [0; 3];
        assert!(read_slice(&memory, source, &mut name));
        assert_eq!(name, [b'A', 0xff, b'B']);
    }

    #[test]
    fn owner_copy_conflict_and_present_unreadable_payload_are_errors() {
        let mut bytes = program();
        bytes[OWNER_COPY] ^= 1;
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            read_row(&memory, 0, 0x1000),
            Err("program owner or province copies mismatch")
        );
        bytes[OWNER_COPY] ^= 1;
        bytes[ACTIVE_FLAG] = 1;
        let memory = AbsentPayload(SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        });
        assert_eq!(
            read_row(&memory, 0, 0x1000),
            Err("program active project unreadable")
        );
    }
}
