
use super::projects_test_kit as kit;
use super::*;
use engine_native::ccountry;
use engine_native_memory::msvc_string;
use engine_projects::{
    nproject_breakthrough_node as node, nproject_cprogram_status as status, nproject_cproject_pool,
    nproject_cproject_state_machine as machine, nproject_cprototype_project_state as prototype,
    nproject_csimple_project_state as simple, nproject_sproject_context as context,
    nproject_sproject_history as history,
};

const GAME: usize = 0x400;
const COUNTRY: usize = 0xfb0;
const PROGRAM: usize = 0x180;
const NODE: usize = 0x28;
const MACHINE: usize = 0x270;

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[derive(Clone, Copy)]
enum Slot {
    Null,
    Country { id: u32, owner: u32 },
    Alias(u32),
}

struct Plan {
    slot: u32,
    id: u32,
    owner: u32,
    projects: u32,
    histories: Vec<u32>,
    programs: u32,
    nodes: u32,
}

pub struct Space {
    pub bytes: Vec<u8>,
    pub game_state: usize,
}

impl Space {
    pub fn memory(&self) -> engine_native_memory::memory::SliceMemory<'_> {
        engine_native_memory::memory::SliceMemory {
            base: self.bytes.as_ptr() as usize,
            bytes: &self.bytes,
        }
    }

    pub fn offset(&self, address: usize) -> usize {
        address - self.bytes.as_ptr() as usize
    }
}

struct Writer {
    bytes: Vec<u8>,
    used: usize,
    machines: usize,
    machine_count: usize,
    machine_step: usize,
    machine_next: usize,
}

impl Writer {
    fn base(&self) -> usize {
        self.bytes.as_ptr() as usize
    }

    fn alloc(&mut self, len: usize) -> usize {
        let at = self.used.next_multiple_of(16);
        self.used = at + len;
        assert!(self.used <= self.bytes.len(), "space size estimate");
        self.base() + at
    }

    fn alloc_machine(&mut self) -> usize {
        let slot = self.machine_next * self.machine_step % self.machine_count;
        self.machine_next += 1;
        self.machines + slot * MACHINE
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        let at = address - self.base();
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn u8(&mut self, address: usize, value: u8) {
        self.put(address, &[value]);
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }

    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }

    fn vtable(&mut self, address: usize, rva: engine_sites::Rva) {
        self.u64(address, (kit::BASE + rva.0) as u64);
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
}

fn slots(countries: u32) -> Vec<Slot> {
    let mut slots = vec![Slot::Null];
    let mut real = 0;
    let mut previous = 0;
    while real < countries {
        let slot = slots.len() as u32;
        if slot.is_multiple_of(50) {
            slots.push(Slot::Null);
        } else if slot.is_multiple_of(83) && previous != 0 {
            slots.push(Slot::Alias(previous));
        } else {
            let id = if slot.is_multiple_of(7) {
                70_000 + slot
            } else {
                slot
            };
            let owner = if slot.is_multiple_of(7) { slot } else { id };
            slots.push(Slot::Country { id, owner });
            previous = slot;
            real += 1;
        }
    }
    slots
}

pub fn space(countries: u32, projects: u32) -> Space {
    let slots = slots(countries);
    let plans: Vec<Plan> = slots
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            let Slot::Country { id, owner } = *entry else {
                return None;
            };
            let seed = mix(u64::from(id));
            let count = if id % 29 == 0 {
                0
            } else {
                projects / 2 + (seed % u64::from(projects + 1)) as u32
            };
            let histories = (0..count)
                .map(|index| {
                    let draw = mix(seed ^ u64::from(index));
                    if draw.is_multiple_of(40) {
                        1 + (draw >> 8) as u32 % 3
                    } else {
                        0
                    }
                })
                .collect();
            Some(Plan {
                slot: slot as u32,
                id,
                owner,
                projects: count,
                histories,
                programs: (seed >> 16) as u32 % 3,
                nodes: (seed >> 24) as u32 % 7,
            })
        })
        .collect();
    let links_count = plans.iter().map(|plan| plan.id).max().unwrap_or(0) + 1;
    let size = GAME
        + slots.len() * 8
        + links_count as usize * 4
        + 64
        + plans
            .iter()
            .map(|plan| {
                let histories: u32 = plan.histories.iter().sum();
                COUNTRY
                    + 0xa0
                    + 0x20
                    + plan.projects as usize * (nproject_cproject::POOL_STRIDE + MACHINE + 16)
                    + histories as usize * history::STRIDE
                    + plan.programs as usize * (PROGRAM + 64 + 8)
                    + (plan.nodes as usize + 1) * NODE
                    + 16 * 8
            })
            .sum::<usize>();
    let machine_count = plans
        .iter()
        .map(|plan| plan.projects as usize)
        .sum::<usize>();
    let mut writer = Writer {
        bytes: vec![0xa5; size],
        used: 0,
        machines: 0,
        machine_count: machine_count.max(1),
        machine_step: (7_919..)
            .find(|step| gcd(*step, machine_count.max(1)) == 1)
            .unwrap(),
        machine_next: 0,
    };
    let game_state = writer.alloc(GAME);
    let table = writer.alloc(slots.len() * 8);
    let links = writer.alloc(links_count as usize * 4);
    let used = writer.used;
    writer.bytes[..used].fill(0);
    writer.machines = writer.alloc(machine_count * MACHINE);
    writer.array(
        game_state + engine_native::cgame_state::COUNTRIES,
        table,
        slots.len() as u32,
    );
    writer.array(
        game_state + engine_native::cgame_state::COUNTRY_LINKS,
        links,
        links_count,
    );
    let mut objects = vec![0_usize; slots.len()];
    let mut program_id = 0_u32;
    for plan in &plans {
        let object = writer.alloc(COUNTRY);
        objects[plan.slot as usize] = object;
        writer.u64(table + plan.slot as usize * 8, object as u64);
        writer.u32(links + plan.slot as usize * 4, plan.slot);
        writer.u32(links + plan.id as usize * 4, plan.slot);
        writer.vtable(object, ccountry::VTABLE_RVA);
        writer.u32(object + ccountry::ID, plan.id);
        let state = writer.alloc(status::SIZE);
        writer.u64(object + ccountry::PROGRAM_STATUS, state as u64);
        writer.vtable(state, status::VTABLE_RVA);
        writer.vtable(
            state + status::SECONDARY_VTABLE,
            status::SECONDARY_VTABLE_RVA,
        );
        writer.u32(state + status::OWNER, plan.id);
        write_pool(&mut writer, state, plan);
        program_id = write_programs(&mut writer, state, plan, program_id);
        write_tree(&mut writer, state, plan);
    }
    for (slot, entry) in slots.iter().enumerate() {
        if let Slot::Alias(target) = *entry {
            writer.u64(table + slot * 8, objects[target as usize] as u64);
            writer.u32(links + slot * 4, target);
        }
    }
    Space {
        bytes: writer.bytes,
        game_state,
    }
}

fn write_pool(writer: &mut Writer, state: usize, plan: &Plan) {
    let pool = writer.alloc(0x20);
    writer.u64(state + status::PROJECT_POOL, pool as u64);
    writer.vtable(pool, nproject_cproject_pool::VTABLE_RVA);
    let first = writer.alloc(plan.projects as usize * nproject_cproject::POOL_STRIDE);
    writer.array(
        pool + nproject_cproject_pool::PROJECTS,
        first,
        plan.projects,
    );
    let states = [
        machine::PROTOTYPE_STATE,
        machine::SIMPLE_STATE_1,
        machine::SIMPLE_STATE_2,
        machine::SIMPLE_STATE_3,
    ];
    for index in 0..plan.projects {
        let seed = mix(u64::from(plan.id) << 32 | u64::from(index));
        let project = first + index as usize * nproject_cproject::POOL_STRIDE;
        writer.vtable(project, nproject_cproject::VTABLE_RVA);
        writer.u32(
            project + nproject_cproject::TYPE,
            nproject_cproject::REFERENCE_TYPE,
        );
        writer.u32(project + nproject_cproject::ID, index + 1);
        writer.u32(
            project + nproject_cproject::DEFINITION_HANDLE,
            seed as u32 % 1000,
        );
        writer.u32(project + nproject_cproject::OWNER, plan.owner);
        writer.u64(
            project + nproject_cproject::PROGRAM_REFERENCE_TYPE,
            mix(seed),
        );
        writer.u8(project + nproject_cproject::STARTED, (seed >> 40) as u8 & 1);
        writer.u8(
            project + nproject_cproject::COMPLETED,
            (seed >> 41) as u8 & 1,
        );
        let state_machine = writer.alloc_machine();
        writer.u64(
            project + nproject_cproject::STATE_MACHINE,
            state_machine as u64,
        );
        writer.vtable(state_machine, machine::VTABLE_RVA);
        let current = states[(seed >> 44) as usize % 4];
        writer.u64(
            state_machine + machine::CURRENT_STATE,
            (state_machine + current) as u64,
        );
        writer.u8(
            state_machine + machine::NOT_COMPLETED,
            (seed >> 46) as u8 & 1,
        );
        for (ordinal, offset) in states.into_iter().enumerate() {
            let at = state_machine + offset;
            let draw = mix(seed ^ ordinal as u64);
            writer.vtable(
                at,
                if ordinal == 0 {
                    prototype::VTABLE_RVA
                } else {
                    simple::VTABLE_RVA
                },
            );
            writer.u32(at + simple::STATE_ENUM, draw as u32 % 5);
            writer.u64(at + simple::PHASE_PROGRESS, draw);
            writer.u32(at + simple::PHASE_TARGET, (draw >> 32) as u32);
        }
        let prototype_state = state_machine + machine::PROTOTYPE_STATE;
        writer.u32(
            prototype_state + prototype::PROTOTYPE_PROGRESS,
            (seed >> 8) as u32 % 101,
        );
        writer.u32(prototype_state + prototype::ITERATIONS, (seed >> 16) as u32);
        let entries = plan.histories[index as usize];
        let data = if entries == 0 {
            0
        } else {
            writer.alloc(entries as usize * history::STRIDE)
        };
        writer.array(project + nproject_cproject::HISTORY, data, entries);
        for ordinal in 0..entries as usize {
            write_history(writer, data + ordinal * history::STRIDE, mix(seed ^ 0xf00d));
        }
    }
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn write_history(writer: &mut Writer, entry: usize, seed: u64) {
    writer.vtable(entry, history::VTABLE_RVA);
    writer.u64(entry + history::PROTOTYPE_REWARD_HANDLE, seed);
    writer.u32(entry + history::DATE, (seed >> 20) as u32);
    let at = entry + history::CONTEXT;
    writer.vtable(at, context::VTABLE_RVA);
    writer.u64(at + context::PROVINCE, mix(seed));
    writer.u64(at + context::CHARACTER_REFERENCE, mix(seed ^ 1));
    for (bit, (present, value)) in [
        (context::COUNTRY_PRESENT, context::COUNTRY_REFERENCE),
        (context::SCIENTIST_PRESENT, context::SCIENTIST_REFERENCE),
        (context::STATE_PRESENT, context::STATE_REFERENCE),
    ]
    .into_iter()
    .enumerate()
    {
        let flag = (seed >> (50 + bit)) as u8 & 1;
        writer.u8(at + present, flag);
        writer.u32(at + value, (seed >> (bit * 8)) as u32);
    }
}

fn write_programs(writer: &mut Writer, state: usize, plan: &Plan, mut id: u32) -> u32 {
    let pointers = writer.alloc(plan.programs as usize * 8);
    writer.array(state + status::PROGRAMS, pointers, plan.programs);
    for index in 0..plan.programs as usize {
        id += 1;
        let seed = mix(u64::from(id) ^ 0xbeef);
        let program = writer.alloc(PROGRAM);
        writer.u64(pointers + index * 8, program as u64);
        writer.vtable(program, nproject_cprogram::VTABLE_RVA);
        writer.u32(
            program + nproject_cprogram::TYPE,
            nproject_cprogram::REFERENCE_TYPE,
        );
        writer.u32(program + nproject_cprogram::ID, id);
        for (owner, province) in [
            (
                nproject_cprogram::OWNER_PRIMARY,
                nproject_cprogram::PROVINCE_PRIMARY,
            ),
            (
                nproject_cprogram::OWNER_COPY,
                nproject_cprogram::PROVINCE_COPY,
            ),
        ] {
            writer.u32(program + owner, plan.id);
            writer.u32(program + province, seed as u32 % 13_000);
        }
        writer.u64(
            program + nproject_cprogram::SCIENTIST_REFERENCE_TYPE,
            mix(seed),
        );
        writer.u32(
            program + nproject_cprogram::UNREAD_PROTOTYPE_REWARDS,
            (seed >> 32) as u32 % 4,
        );
        writer.u8(
            program + nproject_cprogram::ACTIVE_FLAG,
            (seed >> 8) as u8 & 1,
        );
        writer.u32(
            program + nproject_cprogram::ACTIVE_PROJECT_HANDLE,
            (seed >> 12) as u32 % 60,
        );
        let len = (seed >> 20) as usize % 30;
        let text: Vec<u8> = (0..len).map(|at| b'a' + (at % 26) as u8).collect();
        let header = program + nproject_cprogram::BUILDING;
        if len as u64 <= msvc_string::INLINE_CAPACITY {
            writer.put(header, &text);
            writer.u8(header + len, 0);
            writer.u64(header + 24, msvc_string::INLINE_CAPACITY);
        } else {
            let heap = writer.alloc(64);
            writer.put(heap, &text);
            writer.u8(heap + len, 0);
            writer.u64(header, heap as u64);
            writer.u64(header + 24, len as u64);
        }
        writer.u64(header + 16, len as u64);
    }
    id
}

fn write_tree(writer: &mut Writer, state: usize, plan: &Plan) {
    let progress = state + status::BREAKTHROUGH;
    writer.vtable(progress, nproject_cbreakthrough_progress::VTABLE_RVA);
    let header = writer.alloc(NODE);
    writer.u64(
        progress + nproject_cbreakthrough_progress::TREE_HEADER,
        header as u64,
    );
    writer.u64(
        progress + nproject_cbreakthrough_progress::TREE_SIZE,
        u64::from(plan.nodes),
    );
    let nodes: Vec<usize> = (0..plan.nodes).map(|_| writer.alloc(NODE)).collect();
    for (index, &at) in nodes.iter().enumerate() {
        let seed = mix(u64::from(plan.id) ^ index as u64);
        writer.u8(at + node::IS_NIL, 0);
        writer.u32(
            at + node::SPECIALIZATION_HANDLE,
            (index as i32 * 1000 - 3000) as u32,
        );
        writer.u32(at + node::POINTS, seed as u32);
    }
    fn link(writer: &mut Writer, nodes: &[usize], header: usize, parent: usize) -> usize {
        if nodes.is_empty() {
            return header;
        }
        let middle = nodes.len() / 2;
        let at = nodes[middle];
        let left = link(writer, &nodes[..middle], header, at);
        let right = link(writer, &nodes[middle + 1..], header, at);
        writer.u64(at + node::LEFT, left as u64);
        writer.u64(at + node::PARENT, parent as u64);
        writer.u64(at + node::RIGHT, right as u64);
        at
    }
    let root = link(writer, &nodes, header, header);
    writer.u64(
        header + node::LEFT,
        nodes.first().copied().unwrap_or(header) as u64,
    );
    writer.u64(header + node::PARENT, root as u64);
    writer.u64(
        header + node::RIGHT,
        nodes.last().copied().unwrap_or(header) as u64,
    );
    writer.u8(header + node::IS_NIL, 1);
}

fn capacities(countries: usize) -> Capacities {
    let mut capacities = Capacities::default();
    let rows = [countries * 2, countries * 128, countries * 8, countries * 8];
    for (capacity, rows) in capacities.rows.iter_mut().zip(rows) {
        *capacity = (*capacity).max(rows);
    }
    capacities.program_names = countries * 64;
    capacities.program_candidates = countries * 4;
    capacities
}

fn copy(
    memory: &impl Memory,
    game_state: usize,
    capacities: &Capacities,
) -> (Captured, ProjectsJob) {
    let mut job = ProjectsJob::new();
    job.log = RawLog::new(1 << 20, 48 << 20);
    job.census.reserve_exact(capacities.program_candidates);
    job.statuses.reserve_exact(capacities.program_candidates);
    job.pointers.reserve_exact(capacities.program_candidates);
    job.update_capacity_charge();
    let context = ProjectsCopyContext {
        base: kit::BASE,
        game_state,
        active: true,
    };
    assert!(copy_projects_refresh(memory, context, &mut job).raw_ok);
    let mut captured = Captured::with_capacities(capacities);
    captured.decode_projects_payload(kit::BASE, &job, &mut [0; RAW.len()]);
    (captured, job)
}

fn direct(memory: &impl Memory, game_state: usize, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let programs = nproject_cprogram::walk(
        memory,
        kit::BASE,
        game_state,
        &mut captured.candidates,
        &mut captured.programs.rows,
        &mut captured.names.first,
    );
    captured.programs.outcome =
        program_outcome(programs, &captured.programs.rows, &captured.names.first);
    captured.names.len = programs.bytes_len;
    captured.names.required = programs.required_bytes;
    captured.required_candidates = programs.required_candidates;
    let (projects, histories) = nproject_cproject::walk_with_history_buffer(memory, kit::BASE, game_state, engine_native_memory::walk::Buffer::Growing(&mut captured.projects.rows), engine_native_memory::walk::Buffer::Growing(&mut captured.history.rows));
    captured.projects.outcome = bounded(projects, captured.projects.rows.len());
    captured.history.outcome = bounded(histories, captured.history.rows.len());
    captured.breakthroughs.outcome = bounded(
        nproject_cbreakthrough_progress::walk(
            memory,
            kit::BASE,
            game_state,
            &mut captured.breakthroughs.rows,
        ),
        captured.breakthroughs.rows.len(),
    );
    captured
}

fn digest(captured: &Captured) -> u64 {
    fn rows<R: std::fmt::Debug>(text: &mut String, rows: &CaptureRows<R>) {
        use std::fmt::Write;
        let len = rows.outcome.len().min(rows.rows.len());
        write!(text, "{:?}{:?}", rows.outcome, &rows.rows[..len]).unwrap();
    }
    let mut text = String::new();
    rows(&mut text, &captured.programs);
    let mut projects = captured.projects.rows[..captured.projects.outcome.len()].to_vec();
    for project in &mut projects {
        project.source_pointer = 0;
    }
    use std::fmt::Write;
    write!(&mut text, "{:?}{:?}", captured.projects.outcome, projects).unwrap();
    rows(&mut text, &captured.history);
    rows(&mut text, &captured.breakthroughs);
    let names = &captured.names.first[..captured.names.len.min(captured.names.first.len())];
    text.push_str(&format!(
        "{names:?}{}{}",
        captured.names.required, captured.required_candidates
    ));
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn assert_equivalent(space: &Space, capacities: &Capacities, label: &str, golden: u64) {
    let memory = space.memory();
    let reference = direct(&memory, space.game_state, capacities);
    let (split, _) = copy(&memory, space.game_state, capacities);
    kit::assert_matches_direct(&split, &reference);
    check_golden(label, digest(&reference), golden);
}

#[test]
fn scaled_copy_matches_direct_walks_and_the_recorded_digests() {
    for (countries, projects, golden) in [
        (2, 50, 0x20b8_b98a_95a0_e2aa),
        (440, 50, 0xe6d0_2bef_e557_b1b6),
    ] {
        let space = space(countries, projects);
        let label = format!("{countries} countries");
        assert_equivalent(&space, &capacities(countries as usize), &label, golden);
    }
    let mut short = capacities(440);
    short.rows[..4].copy_from_slice(&[100, 5_000, 40, 500]);
    short.program_names = 400;
    let space = space(440, 50);
    assert_equivalent(&space, &short, "short starting storage", 0xe6d0_2bef_e557_b1b6);
    let reference = direct(&space.memory(), space.game_state, &short);
    for outcome in [
        reference.programs.outcome,
        reference.projects.outcome,
        reference.history.outcome,
        reference.breakthroughs.outcome,
    ] {
        assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
    }
}

#[test]
fn live_project_demand_and_owned_decode_growth_keep_mod_sized_pools() {
    let space = space(5_000, 12);
    let memory = space.memory();
    let mut capacity = Capacities::default();
    capacity
        .presize(&memory, kit::BASE, space.game_state)
        .unwrap();
    let (expected, history) = nproject_cproject::walk_with_history_buffer(&memory, kit::BASE, space.game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count);
    let demand = |outcome| match outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len + unread_rows as usize,
        other => panic!("{other:?}"),
    };
    let projects = demand(expected);
    let histories = demand(history);
    assert!(projects > Capacities::default().rows[1]);
    assert!(capacity.rows[1] >= projects && capacity.rows[2] >= histories);
    let mut job = ProjectsJob::default();
    let report = copy_projects_refresh(
        &memory,
        ProjectsCopyContext {
            base: kit::BASE,
            game_state: space.game_state,
            active: true,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    let mut tiny = capacity;
    tiny.rows[..4].fill(0);
    tiny.program_candidates = 0;
    tiny.program_names = 0;
    let mut captured = Captured::with_capacities(&tiny);
    captured.decode_projects_payload(kit::BASE, &job, &mut [0; RAW.len()]);
    let expected = direct(&memory, space.game_state, &capacities(5000));
    kit::assert_matches_direct(&captured, &expected);
    assert!(
        matches!(captured.programs.outcome, Outcome::Rows { len } if len > Capacities::default().rows[0])
    );
    assert!(
        matches!(captured.breakthroughs.outcome, Outcome::Rows { len } if len > Capacities::default().rows[3])
    );
    assert!(captured.names.len > Capacities::default().program_names);
    let later = self::space(5000, 14);
    let later_memory = later.memory();
    assert!(
        copy_projects_refresh(
            &later_memory,
            ProjectsCopyContext {
                base: kit::BASE,
                game_state: later.game_state,
                active: true
            },
            &mut job
        )
        .raw_ok
    );
    captured.decode_projects_payload(kit::BASE, &job, &mut [0; RAW.len()]);
    kit::assert_matches_direct(
        &captured,
        &direct(&later_memory, later.game_state, &capacities(5000)),
    );
}

struct Fault<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    fault: std::ops::Range<usize>,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        if address < self.fault.end && end > self.fault.start {
            return false;
        }
        self.memory.copy(address, output)
    }
}

struct Site {
    label: &'static str,
    fault: usize,
    patch: Option<(usize, u32)>,
}

fn fault_sites(space: &Space) -> Vec<Site> {
    let memory = space.memory();
    let read = |address: usize| engine_native_memory::read::<u64>(&memory, address).unwrap() as usize;
    let read32 = |address: usize| engine_native_memory::read::<u32>(&memory, address).unwrap();
    let table = read(space.game_state + engine_native::cgame_state::COUNTRIES);
    let links = read(space.game_state + engine_native::cgame_state::COUNTRY_LINKS);
    let status_of = |slot: usize| read(read(table + slot * 8) + ccountry::PROGRAM_STATUS);
    let object = read(table + 201 * 8);
    let state = read(object + ccountry::PROGRAM_STATUS);
    let pool = read(state + status::PROJECT_POOL);
    let first = read(pool + nproject_cproject_pool::PROJECTS);
    let project = first + 5 * nproject_cproject::POOL_STRIDE;
    let state_machine = read(project + nproject_cproject::STATE_MACHINE);
    let current = read(state_machine + machine::CURRENT_STATE);
    let header = read(state + status::BREAKTHROUGH + nproject_cbreakthrough_progress::TREE_HEADER);
    let root = read(header + node::PARENT);
    let (history, history_count) = (201..)
        .filter(|&slot| read(table + slot * 8) != 0)
        .flat_map(|slot| {
            let pool = read(status_of(slot) + status::PROJECT_POOL);
            let first = read(pool + nproject_cproject_pool::PROJECTS);
            let count = read32(pool + nproject_cproject_pool::COUNT) as usize;
            (0..count).map(move |index| first + index * nproject_cproject::POOL_STRIDE)
        })
        .map(|project| {
            (
                read(project + nproject_cproject::HISTORY),
                read32(project + nproject_cproject::HISTORY_COUNT),
            )
        })
        .find(|(_, count)| *count > 0)
        .unwrap();
    assert!(history_count > 0);
    let programs = (201..)
        .filter(|&slot| read(table + slot * 8) != 0)
        .map(|slot| status_of(slot) + status::PROGRAMS)
        .find(|&array| read32(array + 12) > 0)
        .map(&read)
        .unwrap();
    let later = 252;
    assert_ne!(read(table + later * 8), 0);
    let patch = (project + nproject_cproject::OWNER, later as u32);
    let stop = read(table + 251 * 8);
    let site = |label, fault| Site {
        label,
        fault,
        patch: None,
    };
    vec![
        site("country object", object),
        site("status", state),
        site("pool", pool),
        site("project header", project + nproject_cproject::ID),
        site("project flags", project + nproject_cproject::STARTED),
        site("machine vtable", state_machine),
        site("machine phase", current + simple::PHASE_PROGRESS),
        site(
            "machine last state",
            state_machine + machine::SIMPLE_STATE_3,
        ),
        site("breakthrough root", root),
        site("breakthrough sentinel", header + node::IS_NIL),
        site("history entry", history + history::DATE),
        site("program pointer array", programs),
        site("links array", links + 201 * 4),
        site(
            "unrecorded machine byte",
            state_machine + machine::SIMPLE_STATE_1 + 0x20,
        ),
        Site {
            label: "owner past a failed census",
            fault: stop,
            patch: Some(patch),
        },
    ]
}

#[test]
fn scaled_faults_match_direct_walks_and_the_recorded_digests() {
    let mut space = space(440, 50);
    let goldens = [
        0xb855_2ee9_c2e1_7946,
        0x37d0_d8db_987c_693e,
        0x7307_b52f_08f0_b288,
        0x5df2_3f5f_5693_a518,
        0xca0e_4b7b_a379_da3f,
        0x6bbd_8b8b_e98d_d5ab,
        0x6bbd_8b8b_e98d_d5ab,
        0x6bbd_8b8b_e98d_d5ab,
        0x645c_45bd_b926_0b24,
        0x645c_45bd_b926_0b24,
        HISTORY_GOLDEN,
        PROGRAMS_GOLDEN,
        LINKS_GOLDEN,
        UNRECORDED_GOLDEN,
        CENSUS_GOLDEN,
    ];
    let sites = fault_sites(&space);
    assert_eq!(sites.len(), goldens.len());
    for (site, golden) in sites.into_iter().zip(goldens) {
        let original = site.patch.map(|(address, value)| {
            let at = space.offset(address);
            let old = space.bytes[at..at + 4].to_vec();
            space.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            (at, old)
        });
        let memory = Fault {
            memory: space.memory(),
            fault: site.fault..site.fault + 1,
        };
        let capacities = capacities(440);
        let reference = direct(&memory, space.game_state, &capacities);
        let (split, _) = copy(&memory, space.game_state, &capacities);
        kit::assert_matches_direct(&split, &reference);
        check_golden(site.label, digest(&reference), golden);
        if let Some((at, old)) = original {
            space.bytes[at..at + 4].copy_from_slice(&old);
        }
    }
}

const HISTORY_GOLDEN: u64 = 0x3d00_8951_bdf6_eb40;
const PROGRAMS_GOLDEN: u64 = 0x345b_47ce_caa9_ee9c;
const LINKS_GOLDEN: u64 = 0xa7fe_1f61_4eee_5ce6;
const CENSUS_GOLDEN: u64 = 0x16e7_6d3d_d939_d851;
const UNRECORDED_GOLDEN: u64 = 0xe6d0_2bef_e557_b1b6;
const BEFORE_UNRECORDED: u64 = 0x73db_88bb_ee04_6cca;

fn check_golden(label: &str, digest: u64, golden: u64) {
    if label == "unrecorded machine byte" {
        assert_ne!(digest, BEFORE_UNRECORDED, "{label}");
    }
    assert_eq!(digest, golden, "{label}");
}
