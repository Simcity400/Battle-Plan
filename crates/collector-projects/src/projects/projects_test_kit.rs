use super::copy_projects_refresh;
use super::{
    Capacities, Captured, ProjectsCopyContext, ProjectsCopyReport, ProjectsJob, bounded,
    program_outcome,
};
use crate::table_capture::CaptureRows;
use engine_native::{ccountry, cgame_state};
use engine_native_memory::msvc_string;
use engine_projects::{
    nproject_breakthrough_node, nproject_cbreakthrough_progress, nproject_cprogram,
    nproject_cprogram_status, nproject_cproject, nproject_cproject_pool,
    nproject_cproject_state_machine, nproject_cprototype_project_state as prototype,
    nproject_csimple_project_state as simple, nproject_sproject_context, nproject_sproject_history,
};

pub const BASE: usize = 0x140000000;
pub const GAME_STATE: usize = 0x10000;
const ADDRESS: usize = 0x10000;
const CTAB: usize = 0x2000;
const LTAB: usize = 0x2100;
pub const C1: usize = 0x3000;
const C2: usize = 0x3100;
pub const S1: usize = 0x4000;
const PARR: usize = 0x5000;
pub const PROG1: usize = 0x5100;
const POOL1: usize = 0x6000;
pub const PROJ1: usize = 0x6200;
pub const MACH1: usize = 0x7000;
pub const HIST1: usize = 0x8000;
const BT_HEAD: usize = 0x9100;
pub const BT_NODE: usize = 0x9200;
pub const HEAP: usize = 0xA000;

pub fn memory(bytes: &[u8]) -> engine_native_memory::memory::SliceMemory<'_> {
    engine_native_memory::memory::SliceMemory {
        base: ADDRESS,
        bytes,
    }
}

fn put<const COUNT: usize>(bytes: &mut [u8], offset: usize, value: [u8; COUNT]) {
    bytes[offset..offset + COUNT].copy_from_slice(&value);
}

fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
    put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
}

fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
    pointer(bytes, offset, data);
    put(bytes, offset + 8, count.to_le_bytes());
    put(bytes, offset + 12, count.to_le_bytes());
}

fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
    assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
    put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
    put(bytes, header + 16, (value.len() as u64).to_le_bytes());
    put(bytes, header + 24, (value.len() as u64).to_le_bytes());
    bytes[payload..payload + value.len()].copy_from_slice(value);
    bytes[payload + value.len()] = 0;
}

fn country(bytes: &mut [u8], slot: u32, id: u32, object: usize, status: usize) {
    pointer(bytes, CTAB + slot as usize * 8, object);
    put(bytes, LTAB + id as usize * 4, id.to_le_bytes());
    put(
        bytes,
        object,
        ((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(bytes, object + ccountry::ID, id.to_le_bytes());
    put(
        bytes,
        object + ccountry::OWNED_STATE_COUNT,
        1_i32.to_le_bytes(),
    );
    pointer(bytes, object + ccountry::PROGRAM_STATUS, status);
    put(
        bytes,
        status,
        ((BASE + nproject_cprogram_status::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        bytes,
        status + nproject_cprogram_status::SECONDARY_VTABLE,
        ((BASE + nproject_cprogram_status::SECONDARY_VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        bytes,
        status + nproject_cprogram_status::OWNER,
        id.to_le_bytes(),
    );
}

fn empty_status(bytes: &mut [u8], status: usize, id: u32, pool: usize) {
    put(
        bytes,
        status,
        ((BASE + nproject_cprogram_status::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        bytes,
        status + nproject_cprogram_status::SECONDARY_VTABLE,
        ((BASE + nproject_cprogram_status::SECONDARY_VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        bytes,
        status + nproject_cprogram_status::OWNER,
        id.to_le_bytes(),
    );
    pointer(bytes, status + nproject_cprogram_status::PROJECT_POOL, pool);
    put(
        bytes,
        pool,
        ((BASE + nproject_cproject_pool::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    array(bytes, pool + nproject_cproject_pool::PROJECTS, 0, 0);
    array(bytes, status + nproject_cprogram_status::PROGRAMS, 0, 0);
    let tree = status + nproject_cprogram_status::BREAKTHROUGH;
    put(
        bytes,
        tree,
        ((BASE + nproject_cbreakthrough_progress::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    let head = tree + 0x40;
    pointer(
        bytes,
        tree + nproject_cbreakthrough_progress::TREE_HEADER,
        head,
    );
    put(
        bytes,
        tree + nproject_cbreakthrough_progress::TREE_SIZE,
        0_u64.to_le_bytes(),
    );
    for link in [0, 8, 16] {
        pointer(bytes, head + link, head);
    }
    bytes[head + nproject_breakthrough_node::IS_NIL] = 1;
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xB000];
    array(&mut bytes, cgame_state::COUNTRIES, CTAB, 3);
    array(&mut bytes, cgame_state::COUNTRY_LINKS, LTAB, 3);
    country(&mut bytes, 1, 1, C1, S1);
    country(&mut bytes, 2, 2, C2, S1 + 0x200);
    pointer(
        &mut bytes,
        S1 + nproject_cprogram_status::PROJECT_POOL,
        POOL1,
    );
    put(
        &mut bytes,
        POOL1,
        ((BASE + nproject_cproject_pool::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        POOL1 + nproject_cproject_pool::PROJECTS,
        PROJ1,
        1,
    );
    array(&mut bytes, S1 + nproject_cprogram_status::PROGRAMS, PARR, 1);
    pointer(&mut bytes, PARR, PROG1);
    put(
        &mut bytes,
        PROG1,
        ((BASE + nproject_cprogram::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::TYPE,
        nproject_cprogram::REFERENCE_TYPE.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::ID,
        5_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::OWNER_PRIMARY,
        1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::PROVINCE_PRIMARY,
        13413_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::OWNER_COPY,
        1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::PROVINCE_COPY,
        13413_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::SCIENTIST_REFERENCE_TYPE,
        0x1122334455667788_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::UNREAD_PROTOTYPE_REWARDS,
        7_i32.to_le_bytes(),
    );
    bytes[PROG1 + nproject_cprogram::ACTIVE_FLAG] = 1;
    put(
        &mut bytes,
        PROG1 + nproject_cprogram::ACTIVE_PROJECT_HANDLE,
        9_u32.to_le_bytes(),
    );
    heap_string(
        &mut bytes,
        PROG1 + nproject_cprogram::BUILDING,
        HEAP,
        b"program-building-01",
    );
    put(
        &mut bytes,
        PROJ1,
        ((BASE + nproject_cproject::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        PROJ1 + nproject_cproject::TYPE,
        nproject_cproject::REFERENCE_TYPE.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROJ1 + nproject_cproject::ID,
        3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROJ1 + nproject_cproject::DEFINITION_HANDLE,
        45_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        PROJ1 + nproject_cproject::OWNER,
        1_u32.to_le_bytes(),
    );
    pointer(&mut bytes, PROJ1 + nproject_cproject::STATE_MACHINE, MACH1);
    put(
        &mut bytes,
        PROJ1 + nproject_cproject::PROGRAM_REFERENCE_TYPE,
        0x99_u64.to_le_bytes(),
    );
    array(&mut bytes, PROJ1 + nproject_cproject::HISTORY, HIST1, 1);
    bytes[PROJ1 + nproject_cproject::STARTED] = 1;
    put(
        &mut bytes,
        MACH1,
        ((BASE + nproject_cproject_state_machine::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    pointer(
        &mut bytes,
        MACH1 + nproject_cproject_state_machine::CURRENT_STATE,
        MACH1 + nproject_cproject_state_machine::SIMPLE_STATE_1,
    );
    bytes[MACH1 + nproject_cproject_state_machine::NOT_COMPLETED] = 1;
    for (index, offset) in [
        nproject_cproject_state_machine::PROTOTYPE_STATE,
        nproject_cproject_state_machine::SIMPLE_STATE_1,
        nproject_cproject_state_machine::SIMPLE_STATE_2,
        nproject_cproject_state_machine::SIMPLE_STATE_3,
    ]
    .into_iter()
    .enumerate()
    {
        let vtable = if index == 0 {
            prototype::VTABLE_RVA
        } else {
            simple::VTABLE_RVA
        };
        put(
            &mut bytes,
            MACH1 + offset,
            ((BASE + vtable.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MACH1 + offset + simple::STATE_ENUM,
            (index as u32 + 10).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        HIST1,
        ((BASE + nproject_sproject_history::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        HIST1 + nproject_sproject_history::PROTOTYPE_REWARD_HANDLE,
        11_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        HIST1 + nproject_sproject_history::OPTION_HANDLE,
        22_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        HIST1 + nproject_sproject_history::DATE,
        333_i32.to_le_bytes(),
    );
    let context = HIST1 + nproject_sproject_history::CONTEXT;
    put(
        &mut bytes,
        context,
        ((BASE + nproject_sproject_context::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        context + nproject_sproject_context::PROVINCE,
        (-5_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        context + nproject_sproject_context::PROGRESS,
        6_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        context + nproject_sproject_context::CHARACTER_REFERENCE,
        0x77_u64.to_le_bytes(),
    );
    bytes[context + nproject_sproject_context::COUNTRY_PRESENT] = 1;
    put(
        &mut bytes,
        context + nproject_sproject_context::COUNTRY_REFERENCE,
        1_u32.to_le_bytes(),
    );
    bytes[context + nproject_sproject_context::STATE_PRESENT] = 1;
    put(
        &mut bytes,
        context + nproject_sproject_context::STATE_REFERENCE,
        9_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        S1 + nproject_cprogram_status::BREAKTHROUGH,
        ((BASE + nproject_cbreakthrough_progress::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    pointer(
        &mut bytes,
        S1 + nproject_cprogram_status::BREAKTHROUGH + nproject_cbreakthrough_progress::TREE_HEADER,
        BT_HEAD,
    );
    put(
        &mut bytes,
        S1 + nproject_cprogram_status::BREAKTHROUGH + nproject_cbreakthrough_progress::TREE_SIZE,
        1_u64.to_le_bytes(),
    );
    pointer(&mut bytes, BT_HEAD, BT_NODE);
    pointer(&mut bytes, BT_HEAD + 8, BT_NODE);
    pointer(&mut bytes, BT_HEAD + 16, BT_NODE);
    bytes[BT_HEAD + nproject_breakthrough_node::IS_NIL] = 1;
    pointer(&mut bytes, BT_NODE, BT_HEAD);
    pointer(&mut bytes, BT_NODE + 8, BT_HEAD);
    pointer(&mut bytes, BT_NODE + 16, BT_HEAD);
    bytes[BT_NODE + nproject_breakthrough_node::IS_NIL] = 0;
    put(
        &mut bytes,
        BT_NODE + nproject_breakthrough_node::SPECIALIZATION_HANDLE,
        77_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BT_NODE + nproject_breakthrough_node::POINTS,
        (-8_i32).to_le_bytes(),
    );
    empty_status(&mut bytes, S1 + 0x200, 2, 0x6100);
    bytes
}

pub const PROGRAMS: usize = 3;
const PROGRAM_SIZE: usize = 0x200;

pub fn program_id(index: usize) -> u32 {
    [7, 5, 6][index]
}

pub fn many_programs_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    let template = bytes[PROG1..PROG1 + PROGRAM_SIZE].to_vec();
    array(&mut bytes, S1 + nproject_cprogram_status::PROGRAMS, PARR, PROGRAMS as u32);
    for index in 0..PROGRAMS {
        let program = PROG1 + index * PROGRAM_SIZE;
        bytes[program..program + PROGRAM_SIZE].copy_from_slice(&template);
        put(&mut bytes, program + nproject_cprogram::ID, program_id(index).to_le_bytes());
        pointer(&mut bytes, PARR + 8 * index, program);
    }
    bytes
}

pub fn add_reserved_alias(bytes: &mut [u8]) {
    const RESERVED: usize = 0x3200;
    array(bytes, cgame_state::COUNTRIES, CTAB, 4);
    pointer(bytes, CTAB, RESERVED);
    pointer(bytes, CTAB + 3 * 8, RESERVED);
    put(
        bytes,
        RESERVED,
        ((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(bytes, RESERVED + ccountry::ID, 0_u32.to_le_bytes());
}

pub fn direct(memory: &impl engine_native_memory::Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let first = nproject_cprogram::walk(
        memory,
        BASE,
        GAME_STATE,
        &mut captured.candidates,
        &mut captured.programs.rows,
        &mut captured.names.first,
    );
    captured.programs.outcome =
        program_outcome(first, &captured.programs.rows, &captured.names.first);
    captured.names.len = first.bytes_len;
    captured.names.required = first.required_bytes;
    captured.required_candidates = first.required_candidates;
    let (projects, histories) = nproject_cproject::walk_with_history_buffer(memory, BASE, GAME_STATE, engine_native_memory::walk::Buffer::Growing(&mut captured.projects.rows), engine_native_memory::walk::Buffer::Growing(&mut captured.history.rows));
    captured.projects.outcome = bounded(projects, captured.projects.rows.len());
    captured.history.outcome = bounded(histories, captured.history.rows.len());
    captured.breakthroughs.outcome = bounded(
        nproject_cbreakthrough_progress::walk(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.breakthroughs.rows,
        ),
        captured.breakthroughs.rows.len(),
    );
    captured
}

pub fn job() -> ProjectsJob {
    let mut job = ProjectsJob::new();
    job.log = crate::raw::RawLog::new(8_192, 1 << 20);
    job.census.reserve_exact(16);
    job.statuses.reserve_exact(16);
    job.pointers.reserve_exact(256);
    job.assignment_references.reserve_exact(1);
    job.update_capacity_charge();
    job
}

pub fn split(
    memory: &impl engine_native_memory::Memory,
    capacities: &Capacities,
) -> (Captured, ProjectsCopyReport, ProjectsJob) {
    let mut captured = Captured::with_capacities(capacities);
    let ctx = ProjectsCopyContext {
        base: BASE,
        game_state: GAME_STATE,
        active: true,
    };
    let mut first = job();
    let report = copy_projects_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; super::RAW.len()];
    captured.decode_projects_payload(BASE, &first, &mut ticks);
    (captured, report, first)
}

fn assert_rows<R: PartialEq + std::fmt::Debug>(
    name: &str,
    split: &CaptureRows<R>,
    direct: &CaptureRows<R>,
) {
    assert_eq!(split.outcome, direct.outcome, "{name} outcome");
    let len = split.outcome.len();
    assert_eq!(direct.outcome.len(), len, "{name} length");
    assert_eq!(&split.rows[..len], &direct.rows[..len], "{name} rows");
}

pub fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_rows("programs", &split.programs, &direct.programs);
    assert_rows("breakthroughs", &split.breakthroughs, &direct.breakthroughs);
    assert_eq!(
        &split.names.first[..split.names.len],
        &direct.names.first[..direct.names.len],
        "program names arena"
    );
    assert_eq!(
        split.names.required, direct.names.required,
        "names required"
    );
    assert_eq!(
        split.required_candidates, direct.required_candidates,
        "required candidates"
    );
}
