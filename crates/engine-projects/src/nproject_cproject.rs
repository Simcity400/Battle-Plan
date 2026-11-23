
use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    nproject_cprogram_status, nproject_cproject_pool,
    nproject_cproject_state_machine::{self as machine, State},
    nproject_sproject_history as history,
    walk::{Buffer, Outcome},
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_f400);
pub const REFERENCE_TYPE: u32 = 86;
pub const POOL_STRIDE: usize = 0x1c0;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const DEFINITION_HANDLE: usize = 0x18;
pub const OWNER: usize = 0x1c;
pub const STATE_MACHINE: usize = 0x28;
pub const PROGRAM_REFERENCE_TYPE: usize = 0x60;
pub const HISTORY: usize = 0x68;
pub const HISTORY_CAPACITY: usize = 0x70;
pub const HISTORY_COUNT: usize = 0x74;
pub const STARTED: usize = 0x1bc;
pub const COMPLETED: usize = 0x1bd;
pub const VARIABLES: usize = 0x138;
pub const PENDING_REWARD: usize = 0x190;
pub const REWARD_TOKEN: usize = 0x08;
pub const REWARD_VTABLE_RVA: Rva = Rva(0x2799268);
const DEFINITION_DATA: usize = 0x20;
const TEMPLATE_DATA: usize = 0x18;
const TEMPLATE_VTABLE_RVA: Rva = Rva(0x29b_dee8);
const TEMPLATE_REWARDS: usize = 0x358;

pub fn pending_reward_ordinal(
    memory: &impl Memory,
    base: usize,
    project: usize,
    pending: usize,
) -> Result<u32, &'static str> {
    let definition = project
        .checked_add(DEFINITION_DATA)
        .and_then(|at| crate::read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|pointer| *pointer != 0)
        .ok_or("pending reward template unreadable")?;
    let template = definition
        .checked_sub(TEMPLATE_DATA)
        .ok_or("pending reward template address invalid")?;
    let expected = engine_sites::at(base, TEMPLATE_VTABLE_RVA)
        .ok_or("pending reward template identity mismatch")?;
    if crate::read::<u64>(memory, template) != Some(expected as u64) {
        return Err("pending reward template identity mismatch");
    }
    let rewards = definition
        .checked_add(TEMPLATE_REWARDS)
        .and_then(|at| Array::read(memory, at, 16))
        .ok_or("pending reward template roster invalid")?;
    for ordinal in 0..rewards.count {
        let address = rewards
            .element(ordinal, 16)
            .ok_or("pending reward template occurrence invalid")?;
        let pointer = crate::read::<u64>(memory, address)
            .ok_or("pending reward template occurrence unreadable")?;
        if pointer == pending as u64 {
            return Ok(ordinal);
        }
    }
    Err("pending reward object is absent from its template")
}

const BATCH: usize = 16;
const HEADER_LEN: usize = HISTORY_COUNT + 4;
const RECORD_SIZE: usize = HEADER_LEN + 2;
const PROJECT_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: HEADER_LEN,
    },
    RecordField {
        source_offset: STARTED,
        destination_offset: HEADER_LEN,
        len: 2,
    },
];
const PROJECT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROJECT_FIELDS, RECORD_SIZE).expect("valid record layout");
const PROJECT_HEADER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(std::slice::from_ref(&PROJECT_FIELDS[0]), RECORD_SIZE)
        .expect("valid record layout");

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source_pointer: usize,
    pub owner: u32,
    pub local_id: u32,
    pub definition_id: u32,
    pub program_reference: u64,
    pub started: u8,
    pub completed: u8,
    pub state: State,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.owner) << 32) | u64::from(self.local_id)
    }
}

const POOL_HEAD_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: nproject_cproject_pool::PROJECTS + 16,
}];
const POOL_HEAD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POOL_HEAD_FIELDS, nproject_cproject_pool::PROJECTS + 16)
        .expect("valid project pool head layout");

fn for_each_pool<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    select: &mut dyn FnMut(u32) -> bool,
    mut visit: impl FnMut(
        &ccountry::Reader<'_, M>,
        ccountry::Country,
        Array,
    ) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let pool_vtable = engine_sites::at(base, nproject_cproject_pool::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut found_country = false;
    let mut batch_countries = [ccountry::Country {
        pointer: 0,
        canonical_slot: 0,
        id: 0,
    }; nproject_cprogram_status::BATCH];
    let mut statuses = [nproject_cprogram_status::BatchedStatus::EMPTY;
        nproject_cprogram_status::BATCH];
    let mut pool_ptrs = [0_usize; nproject_cprogram_status::BATCH];
    let mut pool_slots = [0_usize; nproject_cprogram_status::BATCH];
    let mut pool_bytes =
        [0_u8; nproject_cprogram_status::BATCH * (nproject_cproject_pool::PROJECTS + 16)];
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
            if !select(country.id) {
                continue;
            }
            batch_countries[n] = country;
            n += 1;
        }
        if n == 0 {
            continue;
        }
        nproject_cprogram_status::read_batch(
            memory,
            base,
            &countries,
            &batch_countries[..n],
            &mut statuses,
        )?;
        let mut pool_count = 0_usize;
        for (i, entry) in statuses[..n].iter().enumerate() {
            if let Some(block) = &entry.block
                && let Ok(pool) = nproject_cprogram_status::pool_from_block(block)
                && pool
                    .checked_add(nproject_cproject_pool::PROJECTS + 16)
                    .is_some()
            {
                pool_ptrs[pool_count] = pool;
                pool_slots[pool_count] = i;
                pool_count += 1;
            }
        }
        let head_len = nproject_cproject_pool::PROJECTS + 16;
        let pools_copied = if pool_count > 0 {
            memory.copy_records(
                RecordSources::Pointers(&pool_ptrs[..pool_count]),
                &POOL_HEAD_LAYOUT,
                &mut pool_bytes[..pool_count * head_len],
            )
        } else {
            0
        };
        let mut pool_index = [usize::MAX; nproject_cprogram_status::BATCH];
        for k in 0..pools_copied {
            pool_index[pool_slots[k]] = k;
        }
        for (i, entry) in statuses[..n].iter().enumerate() {
            let projects = if pool_index[i] != usize::MAX {
                let k = pool_index[i];
                let slice = &pool_bytes[k * head_len..(k + 1) * head_len];
                if u64::from_le_bytes(slice[..8].try_into().unwrap()) != pool_vtable {
                    return Err("project pool vtable mismatch");
                }
                Array::from_header(
                    slice[nproject_cproject_pool::PROJECTS..head_len]
                        .try_into()
                        .unwrap(),
                    POOL_STRIDE,
                )
                .ok_or("project pool array unreadable or invalid")?
            } else {
                nproject_cproject_pool::from_status(memory, base, entry.status)?
            };
            visit(&countries, entry.country, projects)?;
        }
    }
    if !found_country {
        return Err("project country roster contains no countries");
    }
    Ok(())
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut Vec<Row>) -> Outcome {
    walk_tables(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        None,
        &mut |_| true,
    )
    .0
}

pub fn count_with_history(memory: &impl Memory, base: usize, game_state: usize) -> (Outcome, Outcome) {
    walk_with_history_buffer(memory, base, game_state, Buffer::Count, Buffer::Count)
}

#[cfg(test)]
pub fn walk_with_history(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    projects: &mut Vec<Row>,
    histories: &mut Vec<history::Row>,
) -> (Outcome, Outcome) {
    walk_with_history_buffer(
        memory,
        base,
        game_state,
        Buffer::Growing(projects),
        Buffer::Growing(histories),
    )
}

pub fn walk_with_history_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    projects: Buffer<'_, Row>,
    histories: Buffer<'_, history::Row>,
) -> (Outcome, Outcome) {
    walk_tables(
        memory,
        base,
        game_state,
        projects,
        Some(histories),
        &mut |_| true,
    )
}

pub fn walk_with_history_selected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    projects: Buffer<'_, Row>,
    histories: Buffer<'_, history::Row>,
    select: &mut dyn FnMut(u32) -> bool,
) -> (Outcome, Outcome) {
    walk_tables(memory, base, game_state, projects, Some(histories), select)
}

pub struct PartitionReport {
    pub outcomes: (Outcome, Outcome),
    pub census_error: Option<&'static str>,
}

pub fn walk_partition(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    projects: Buffer<'_, Row>,
    histories: Buffer<'_, history::Row>,
    select: &mut dyn FnMut(u32) -> bool,
) -> PartitionReport {
    walk_tables_report(
        memory,
        base,
        game_state,
        projects,
        Some(histories),
        select,
        false,
    )
}

fn walk_tables(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    projects: Buffer<'_, Row>,
    histories: Option<Buffer<'_, history::Row>>,
    select: &mut dyn FnMut(u32) -> bool,
) -> (Outcome, Outcome) {
    walk_tables_report(memory, base, game_state, projects, histories, select, true).outcomes
}

fn walk_tables_report(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut projects: Buffer<'_, Row>,
    mut histories: Option<Buffer<'_, history::Row>>,
    select: &mut dyn FnMut(u32) -> bool,
    check_duplicates: bool,
) -> PartitionReport {
    let mut project_len = 0_usize;
    let mut project_unread = 0_u32;
    let mut project_error = None;
    let mut history_len = 0_usize;
    let mut history_unread = 0_u32;
    let mut history_error = None;
    let mut headers = [0; RECORD_SIZE * BATCH];
    let mut machines = [0; machine::FIELDS_SIZE * BATCH];
    let mut phases = [0; machine::PHASE_SIZE * BATCH];
    let mut pointers = [0; BATCH];
    let mut machine_pointers = [0; BATCH];
    let mut phase_pointers = [0; BATCH];
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("project vtable overflow")? as u64;
        let decoder = machine::Decoder::new(base)?;
        for_each_pool(
            memory,
            base,
            game_state,
            select,
            |countries, country, pool| {
                if project_error.is_none() {
                    projects.ensure(
                        project_len
                            .checked_add(pool.count as usize)
                            .ok_or("project row count overflow")?,
                    )?;
                }
                let mut index = 0;
                let mut authenticated_owner = country.id;
                while index < pool.count {
                    let capture_project = project_error.is_none() && project_len < projects.len();
                    let mut count = BATCH.min((pool.count - index) as usize);
                    if capture_project {
                        count = count.min(projects.len() - project_len);
                    }
                    for (ordinal, pointer) in pointers[..count].iter_mut().enumerate() {
                        *pointer = pool
                            .element(index + ordinal as u32, POOL_STRIDE)
                            .ok_or("project address overflow")?;
                    }
                    let layout = if capture_project {
                        &PROJECT_LAYOUT
                    } else {
                        &PROJECT_HEADER_LAYOUT
                    };
                    let fields = layout.fields();
                    let copied = memory.copy_records(
                        RecordSources::Pointers(&pointers[..count]),
                        layout,
                        &mut headers[..count * RECORD_SIZE],
                    );
                    if copied > count * fields.len() {
                        return Err("project batch copy count invalid");
                    }
                    if copied < count * fields.len() && copied.is_multiple_of(fields.len()) {
                        return Err("project identity unreadable");
                    }
                    let header_count = copied.div_ceil(fields.len());
                    if header_count == 0 {
                        return Err("project identity unreadable");
                    }
                    if copied < count * fields.len() {
                        project_error = Some("project lifecycle unreadable");
                    }
                    for ordinal in 0..header_count {
                        let bytes = &headers[ordinal * RECORD_SIZE..(ordinal + 1) * RECORD_SIZE];
                        if u64_at(bytes, 0) != expected {
                            return Err("project vtable mismatch");
                        }
                        if u32_at(bytes, TYPE) != REFERENCE_TYPE {
                            return Err("project reference type mismatch");
                        }
                        if u32_at(bytes, ID) == 0 {
                            return Err("project local id is zero");
                        }
                        let owner = u32_at(bytes, OWNER);
                        if owner != authenticated_owner {
                            if countries.resolve(owner)?.pointer != country.pointer {
                                return Err("project pool owner mismatch");
                            }
                            authenticated_owner = owner;
                        }
                        machine_pointers[ordinal] =
                            usize::try_from(u64_at(bytes, STATE_MACHINE))
                                .map_err(|_| "project state address overflow")?;
                    }
                    if capture_project && project_error.is_none() {
                        let copied = memory.copy_records(
                            RecordSources::Pointers(&machine_pointers[..header_count]),
                            &machine::LAYOUT,
                            &mut machines[..header_count * machine::FIELDS_SIZE],
                        );
                        let copied = copied == header_count * machine::FIELDS.len() && {
                            for (ordinal, source) in
                                phase_pointers[..header_count].iter_mut().enumerate()
                            {
                                *source = machine::phase_source(
                                    machine_pointers[ordinal],
                                    machine_fields(&machines, ordinal),
                                );
                            }
                            memory.copy_records(
                                RecordSources::Pointers(&phase_pointers[..header_count]),
                                &machine::PHASE_LAYOUT,
                                &mut phases[..header_count * machine::PHASE_SIZE],
                            ) == header_count
                        };
                        if !copied {
                            project_error = Some("project state machine unreadable");
                        }
                    }
                    for ordinal in 0..header_count {
                        let bytes = &headers[ordinal * RECORD_SIZE..(ordinal + 1) * RECORD_SIZE];
                        let owner = u32_at(bytes, OWNER);
                        let id = u32_at(bytes, ID);
                        if project_error.is_none() {
                            if !capture_project {
                                project_unread = project_unread
                                    .checked_add(1)
                                    .ok_or("project row count overflow")?;
                            } else {
                                let row = (|| {
                                    let flags = &bytes[HEADER_LEN..];
                                    if flags.iter().any(|&flag| flag > 1) {
                                        return Err("project lifecycle flag invalid");
                                    }
                                    let phase = ordinal * machine::PHASE_SIZE;
                                    Ok(Row {
                                        source_pointer: pointers[ordinal],
                                        owner,
                                        local_id: id,
                                        definition_id: u32_at(bytes, DEFINITION_HANDLE),
                                        program_reference: u64_at(bytes, PROGRAM_REFERENCE_TYPE),
                                        started: flags[0],
                                        completed: flags[1],
                                        state: decoder.decode(
                                            machine_pointers[ordinal],
                                            machine_fields(&machines, ordinal),
                                            phases[phase..phase + machine::PHASE_SIZE]
                                                .try_into()
                                                .unwrap(),
                                        )?,
                                    })
                                })();
                                match row {
                                    Ok(row) => {
                                        projects[project_len] = row;
                                        project_len += 1;
                                    }
                                    Err(reason) => project_error = Some(reason),
                                }
                            }
                        }
                        if let Some(output) = histories.as_mut().filter(|_| history_error.is_none())
                        {
                            let result = (|| {
                                let array = history_array(bytes[..HEADER_LEN].try_into().unwrap())?;
                                output.ensure(
                                    history_len
                                        .checked_add(array.count as usize)
                                        .ok_or("project history row count overflow")?,
                                )?;
                                let read_count =
                                    (array.count as usize).min(output.len() - history_len) as u32;
                                history_unread = history_unread
                                    .checked_add(array.count - read_count)
                                    .ok_or("project history omitted row count overflow")?;
                                for ordinal in 0..read_count {
                                    let address = array
                                        .element(ordinal, history::STRIDE)
                                        .ok_or("project history row address overflow")?;
                                    output[history_len] = history::read_row(
                                        memory, base, address, owner, id, ordinal,
                                    )?;
                                    history_len += 1;
                                }
                                Ok(())
                            })();
                            if let Err(reason) = result {
                                history_error = Some(reason);
                            }
                        }
                    }
                    index += header_count as u32;
                }
                Ok(())
            },
        )
    })();
    if let Err(reason) = result {
        return PartitionReport {
            outcomes: (
                Outcome::Unavailable { reason },
                Outcome::Unavailable { reason },
            ),
            census_error: Some(reason),
        };
    }
    if project_error.is_none() {
        projects[..project_len].sort_unstable_by_key(|row| row.key());
        if check_duplicates
            && projects[..project_len]
                .windows(2)
                .any(|pair| pair[0].key() == pair[1].key())
        {
            project_error = Some("duplicate country-local project id");
        }
    }
    if let Some(mut histories) = histories.filter(|_| history_error.is_none()) {
        histories[..history_len].sort_unstable_by_key(|row| (row.owner, row.project, row.ordinal));
        if check_duplicates
            && histories[..history_len].windows(2).any(|pair| {
                (pair[0].owner, pair[0].project, pair[0].ordinal)
                    == (pair[1].owner, pair[1].project, pair[1].ordinal)
            })
        {
            history_error = Some("duplicate country-local project history row");
        }
    }
    PartitionReport {
        outcomes: (
            outcome(project_len, project_unread, project_error),
            outcome(history_len, history_unread, history_error),
        ),
        census_error: None,
    }
}

fn outcome(len: usize, unread_rows: u32, error: Option<&'static str>) -> Outcome {
    match error {
        Some(reason) => Outcome::Unavailable { reason },
        None if unread_rows != 0 => Outcome::Truncated { len, unread_rows },
        None => Outcome::Rows { len },
    }
}

fn machine_fields(machines: &[u8], ordinal: usize) -> &[u8; machine::FIELDS_SIZE] {
    machines[ordinal * machine::FIELDS_SIZE..(ordinal + 1) * machine::FIELDS_SIZE]
        .try_into()
        .unwrap()
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn history_array(bytes: &[u8; HISTORY_COUNT + 4]) -> Result<Array, &'static str> {
    let data =
        usize::try_from(u64_at(bytes, HISTORY)).map_err(|_| "project history address overflow")?;
    let capacity = u32_at(bytes, HISTORY_CAPACITY);
    let count = u32_at(bytes, HISTORY_COUNT);
    if capacity > i32::MAX as u32
        || count > capacity
        || (count != 0 && data == 0)
        || (count as usize)
            .checked_mul(history::STRIDE)
            .and_then(|len| data.checked_add(len))
            .is_none()
    {
        return Err("project history array unreadable or invalid");
    }
    Ok(Array { data, count })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::{
        cgame_state, memory::SliceMemory, nproject_cproject_state_machine,
        nproject_cprototype_project_state as prototype, nproject_csimple_project_state as simple,
        nproject_sproject_context,
    };

    const GAME: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const COUNTRY: usize = 0x3000;
    const STATUS: usize = 0x4800;
    const POOL: usize = 0x4900;
    const PROJECT: usize = 0x5000;
    const MACHINE: usize = 0x6000;
    const HISTORY_ROW: usize = 0x7000;
    const MANY_PROJECTS: usize = 0x10000;
    const MANY_MACHINES: usize = 0x20000;
    const MANY_HISTORIES: usize = 0x40000;
    const MANY_COUNT: usize = BATCH * 2 + 1;

    #[test]
    fn pending_reward_identity_preserves_template_occurrence_instead_of_repeated_token() {
        let mut bytes = vec![0; 0x8000];
        let template = 0x1000;
        let roster = 0x2000;
        put64(
            &mut bytes,
            PROJECT + DEFINITION_DATA,
            (template + TEMPLATE_DATA) as u64,
        );
        put64(&mut bytes, template, TEMPLATE_VTABLE_RVA.0 as u64);
        array(
            &mut bytes,
            template + TEMPLATE_DATA + TEMPLATE_REWARDS,
            roster,
            3,
        );
        for (ordinal, pointer) in [0x3000, 0x4000, 0x5000].into_iter().enumerate() {
            put64(&mut bytes, roster + ordinal * 16, pointer as u64);
            put32(&mut bytes, pointer + REWARD_TOKEN, 77);
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(pending_reward_ordinal(&memory, 0, PROJECT, 0x4000), Ok(1));
        assert!(pending_reward_ordinal(&memory, 0, PROJECT, 0x6000).is_err());
        put64(&mut bytes, template, 0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(pending_reward_ordinal(&memory, 0, PROJECT, 0x4000).is_err());
    }

    fn put32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
        put64(bytes, address, data as u64);
        put32(bytes, address + 8, count);
        put32(bytes, address + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, GAME + cgame_state::COUNTRIES, COUNTRIES, 4);
        array(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
        for slot in [1, 3] {
            put64(&mut bytes, COUNTRIES + slot * 8, COUNTRY as u64);
            put32(&mut bytes, LINKS + slot * 4, 1);
        }
        put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, COUNTRY + ccountry::ID, 3);
        put64(
            &mut bytes,
            COUNTRY + ccountry::PROGRAM_STATUS,
            STATUS as u64,
        );
        put64(
            &mut bytes,
            STATUS,
            nproject_cprogram_status::VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            STATUS + 8,
            nproject_cprogram_status::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put32(&mut bytes, STATUS + nproject_cprogram_status::OWNER, 3);
        put64(
            &mut bytes,
            STATUS + nproject_cprogram_status::PROJECT_POOL,
            POOL as u64,
        );
        put64(
            &mut bytes,
            POOL,
            nproject_cproject_pool::VTABLE_RVA.0 as u64,
        );
        array(
            &mut bytes,
            POOL + nproject_cproject_pool::PROJECTS,
            PROJECT,
            1,
        );
        put64(&mut bytes, PROJECT, VTABLE_RVA.0 as u64);
        put32(&mut bytes, PROJECT + TYPE, REFERENCE_TYPE);
        put32(&mut bytes, PROJECT + ID, 17);
        put32(&mut bytes, PROJECT + DEFINITION_HANDLE, 45);
        put32(&mut bytes, PROJECT + OWNER, 1);
        put64(&mut bytes, PROJECT + STATE_MACHINE, MACHINE as u64);
        bytes[PROJECT + STARTED] = 1;
        array(&mut bytes, PROJECT + HISTORY, HISTORY_ROW, 1);
        put64(
            &mut bytes,
            MACHINE,
            nproject_cproject_state_machine::VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            MACHINE + nproject_cproject_state_machine::CURRENT_STATE,
            (MACHINE + nproject_cproject_state_machine::SIMPLE_STATE_2) as u64,
        );
        bytes[MACHINE + nproject_cproject_state_machine::NOT_COMPLETED] = 1;
        for (index, offset) in [0x18, 0xe8, 0x168, 0x1e8].into_iter().enumerate() {
            let vtable = if index == 0 {
                prototype::VTABLE_RVA
            } else {
                simple::VTABLE_RVA
            };
            put64(&mut bytes, MACHINE + offset, vtable.0 as u64);
            put32(
                &mut bytes,
                MACHINE + offset + simple::STATE_ENUM,
                index as u32,
            );
        }
        put64(&mut bytes, MACHINE + 0x168 + simple::PHASE_PROGRESS, 7);
        put64(&mut bytes, HISTORY_ROW, history::VTABLE_RVA.0 as u64);
        put64(
            &mut bytes,
            HISTORY_ROW + history::CONTEXT,
            nproject_sproject_context::VTABLE_RVA.0 as u64,
        );
        bytes
    }

    fn many_projects() -> Vec<u8> {
        let mut bytes = fixture();
        let project = bytes[PROJECT..PROJECT + POOL_STRIDE].to_vec();
        let machine = bytes[MACHINE..MACHINE + nproject_cproject_state_machine::SIZE].to_vec();
        let reward = bytes[HISTORY_ROW..HISTORY_ROW + history::STRIDE].to_vec();
        bytes.resize(0x50000, 0);
        array(
            &mut bytes,
            POOL + nproject_cproject_pool::PROJECTS,
            MANY_PROJECTS,
            MANY_COUNT as u32,
        );
        for index in 0..MANY_COUNT {
            let project_at = MANY_PROJECTS + index * POOL_STRIDE;
            let machine_at = MANY_MACHINES + index * nproject_cproject_state_machine::SIZE;
            let history_at = MANY_HISTORIES + index * history::STRIDE;
            bytes[project_at..project_at + project.len()].copy_from_slice(&project);
            bytes[machine_at..machine_at + machine.len()].copy_from_slice(&machine);
            bytes[history_at..history_at + reward.len()].copy_from_slice(&reward);
            put32(&mut bytes, project_at + ID, 17 + index as u32);
            put64(&mut bytes, project_at + STATE_MACHINE, machine_at as u64);
            array(&mut bytes, project_at + HISTORY, history_at, 1);
            put64(
                &mut bytes,
                machine_at + nproject_cproject_state_machine::CURRENT_STATE,
                (machine_at + nproject_cproject_state_machine::SIMPLE_STATE_2) as u64,
            );
            put64(
                &mut bytes,
                machine_at
                    + nproject_cproject_state_machine::SIMPLE_STATE_2
                    + simple::PHASE_PROGRESS,
                (i64::MIN + index as i64) as u64,
            );
            put32(
                &mut bytes,
                history_at + history::DATE,
                (i32::MIN + index as i32) as u32,
            );
        }
        bytes
    }

    #[test]
    fn a_selected_walk_reads_only_the_selected_pools() {
        struct Reads<'a> {
            memory: SliceMemory<'a>,
            status: std::cell::Cell<bool>,
        }
        impl Memory for Reads<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if (STATUS..STATUS + 0x100).contains(&address) {
                    self.status.set(true);
                }
                self.memory.copy(address, destination)
            }
        }
        let bytes = fixture();
        let memory = Reads {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            status: Default::default(),
        };
        let mut full = (vec![], vec![]);
        let expected = walk_with_history(&memory.memory, 0, GAME, &mut full.0, &mut full.1);
        let mut projects = vec![];
        let mut histories = vec![];
        let mut selected = |id: u32| id == 3;
        assert_eq!(
            walk_with_history_selected(
                &memory,
                0,
                GAME,
                Buffer::Growing(&mut projects),
                Buffer::Growing(&mut histories),
                &mut selected,
            ),
            expected
        );
        assert!(memory.status.get());
        assert_eq!((projects, histories), full);
        let memory = Reads {
            status: Default::default(),
            ..memory
        };
        let (mut projects, mut histories) = (vec![], vec![]);
        assert_eq!(
            walk_with_history_selected(
                &memory,
                0,
                GAME,
                Buffer::Growing(&mut projects),
                Buffer::Growing(&mut histories),
                &mut |_| false,
            ),
            (Outcome::Rows { len: 0 }, Outcome::Rows { len: 0 })
        );
        assert!(!memory.status.get());
    }

    #[test]
    fn multiple_batches_preserve_signed_fields_and_history_joins() {
        let bytes = many_projects();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut projects = vec![Row::default(); MANY_COUNT];
        let mut histories = vec![history::Row::default(); MANY_COUNT];
        assert_eq!(
            walk_with_history(&memory, 0, GAME, &mut projects, &mut histories),
            (
                Outcome::Rows { len: MANY_COUNT },
                Outcome::Rows { len: MANY_COUNT },
            )
        );
        for (index, (project, history)) in projects.iter().zip(&histories).enumerate() {
            assert_eq!(project.owner, 1);
            assert_eq!(project.local_id, 17 + index as u32);
            assert_eq!(project.state.phase_progress, i64::MIN + index as i64);
            assert_eq!(history.owner, project.owner);
            assert_eq!(history.project, project.local_id);
            assert_eq!(history.ordinal, 0);
            assert_eq!(history.date, i32::MIN + index as i32);
        }
    }

    #[test]
    fn partial_gather_failures_are_not_retried_and_keep_independent_history() {
        struct FaultMemory<'a> {
            memory: SliceMemory<'a>,
            fault: usize,
            failures: Cell<usize>,
        }
        impl Memory for FaultMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= self.fault && self.fault < address + destination.len() {
                    self.failures.set(self.failures.get() + 1);
                    destination[0] = 0xff;
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }
        let bytes = many_projects();
        for (fault, reason, history_readable) in [
            (
                MANY_PROJECTS + 8 * POOL_STRIDE + STARTED,
                "project lifecycle unreadable",
                true,
            ),
            (
                MANY_MACHINES + 8 * nproject_cproject_state_machine::SIZE,
                "project state machine unreadable",
                true,
            ),
            (
                MANY_PROJECTS + 8 * POOL_STRIDE,
                "project identity unreadable",
                false,
            ),
        ] {
            for growing in [false, true] {
                let memory = FaultMemory {
                    memory: SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    },
                    fault,
                    failures: Cell::new(0),
                };
                let mut projects = vec![Row::default(); if growing { 0 } else { MANY_COUNT }];
                let mut histories =
                    vec![history::Row::default(); if growing { 0 } else { MANY_COUNT }];
                let result = walk_with_history_buffer(
                    &memory,
                    0,
                    GAME,
                    Buffer::Growing(&mut projects),
                    Buffer::Growing(&mut histories),
                );
                assert_eq!(result.0, Outcome::Unavailable { reason });
                assert_eq!(
                    result.1,
                    if history_readable {
                        Outcome::Rows { len: MANY_COUNT }
                    } else {
                        Outcome::Unavailable { reason }
                    }
                );
                assert_eq!(memory.failures.get(), 1);
                if history_readable {
                    for (index, history) in histories[..MANY_COUNT].iter().enumerate() {
                        assert_eq!(history.project, 17 + index as u32);
                        assert_eq!(history.date, i32::MIN + index as i32);
                    }
                }
            }
        }
    }

    #[test]
    fn equivalent_owner_alias_is_authenticated_and_preserved_in_both_tables() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut projects = vec![Row::default(); 1];
        let mut histories = vec![history::Row::default(); 1];
        assert_eq!(
            walk_with_history(&memory, 0, GAME, &mut projects, &mut histories),
            (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
        );
        assert_eq!(
            (
                projects[0].owner,
                projects[0].local_id,
                projects[0].state.phase_progress
            ),
            (1, 17, 7)
        );
        assert_eq!(
            (
                histories[0].owner,
                histories[0].project,
                histories[0].ordinal
            ),
            (1, 17, 0)
        );
    }

    #[test]
    fn state_and_history_failures_are_independent_but_invalid_owner_is_not_invented() {
        for (at, failed) in [(MACHINE, 0), (HISTORY_ROW, 1), (PROJECT + OWNER, 2)] {
            let mut bytes = fixture();
            put32(&mut bytes, at, 0);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let result = walk_with_history(
                &memory,
                0,
                GAME,
                &mut vec![Row::default(); 1],
                &mut vec![history::Row::default(); 1],
            );
            match failed {
                0 => {
                    assert!(matches!(result.0, Outcome::Unavailable { .. }));
                    assert_eq!(result.1, Outcome::Rows { len: 1 });
                }
                1 => {
                    assert_eq!(result.0, Outcome::Rows { len: 1 });
                    assert!(matches!(result.1, Outcome::Unavailable { .. }));
                }
                _ => {
                    assert!(matches!(result.0, Outcome::Unavailable { .. }));
                    assert!(matches!(result.1, Outcome::Unavailable { .. }));
                }
            }
        }
    }

    #[test]
    fn each_count_probe_counts_its_table_without_reading_its_state_or_history() {
        let mut bytes = fixture();
        put64(&mut bytes, PROJECT + STATE_MACHINE, u64::MAX);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_with_history_buffer(
                &memory,
                0,
                GAME,
                Buffer::Count,
                Buffer::Growing(&mut vec![history::Row::default(); 1]),
            ),
            (
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                Outcome::Rows { len: 1 }
            )
        );
        let mut bytes = fixture();
        put64(&mut bytes, PROJECT + HISTORY, 0x9000);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_with_history_buffer(
                &memory,
                0,
                GAME,
                Buffer::Growing(&mut vec![Row::default(); 1]),
                Buffer::Count,
            ),
            (
                Outcome::Rows { len: 1 },
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                }
            )
        );
    }
}
