
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
};
use engine_native::{cfleet, cnaval_combat, cprovince, ctask_force, cunit};
use crate::{cloop_history, loop_history_record_fields, loop_history_window};
use engine_native_air::cstrategic_air;
use engine_native_parts::cstrategic_region;
use engine_sites::Rva;

pub const ENEMY_SHIPS_SUNK: usize = 43;
pub const OUR_FIGHTERS_LOST: usize = 3;
pub const CHANNELS: [usize; 9] = [
    ENEMY_SHIPS_SUNK,
    OUR_FIGHTERS_LOST,
    10,
    17,
    51,
    58,
    44,
    24,
    65,
];
const FIRST_WINDOW: usize = 0x10;
pub const SUM_LIMIT: i64 = 0x53e2_d620_fb40;
const BATCH: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskForceCombat {
    pub province: u32,
    pub region: u32,
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

const QWORD: [RecordField; 1] = [field(0, 0, 8)];
const QWORD_LAYOUT: RecordLayout<'static> = RecordLayout::new(&QWORD, 8).expect("valid layout");
const FLEET_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cfleet::TASK_FORCES, 8, 16)];
const FLEET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FLEET_FIELDS, 24).expect("valid layout");
const TASK_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(ctask_force::SECONDARY_BASE, 8, 8),
    field(cunit::COMBAT_REFERENCE_DATA, 16, 16),
];
const TASK_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TASK_FIELDS, 32).expect("valid layout");
const COMBAT_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cnaval_combat::PROVINCE, 8, 8)];
const COMBAT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COMBAT_FIELDS, 16).expect("valid layout");
const PROVINCE_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(cprovince::ID, 8, 4),
    field(cprovince::STRATEGIC_REGION, 12, 8),
];
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, 20).expect("valid layout");
const REGION_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cstrategic_region::ID, 8, 4)];
const REGION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&REGION_FIELDS, 12).expect("valid layout");
const HISTORY_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(cloop_history::FIELD_COUNT, 8, 4),
    field(FIRST_WINDOW, 12, 8),
];
const HISTORY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&HISTORY_FIELDS, 20).expect("valid layout");
const WINDOW_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(loop_history_window::RECORDS, 8, 16),
    field(loop_history_window::CAPACITY, 24, 9),
];
const WINDOW_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&WINDOW_FIELDS, 33).expect("valid layout");
const CHANNEL_FIELDS: [RecordField; 9] = [
    field(CHANNELS[0] * loop_history_record_fields::VALUE_STRIDE, 0, 8),
    field(CHANNELS[1] * loop_history_record_fields::VALUE_STRIDE, 8, 8),
    field(
        CHANNELS[2] * loop_history_record_fields::VALUE_STRIDE,
        16,
        8,
    ),
    field(
        CHANNELS[3] * loop_history_record_fields::VALUE_STRIDE,
        24,
        8,
    ),
    field(
        CHANNELS[4] * loop_history_record_fields::VALUE_STRIDE,
        32,
        8,
    ),
    field(
        CHANNELS[5] * loop_history_record_fields::VALUE_STRIDE,
        40,
        8,
    ),
    field(
        CHANNELS[6] * loop_history_record_fields::VALUE_STRIDE,
        48,
        8,
    ),
    field(
        CHANNELS[7] * loop_history_record_fields::VALUE_STRIDE,
        56,
        8,
    ),
    field(
        CHANNELS[8] * loop_history_record_fields::VALUE_STRIDE,
        64,
        8,
    ),
];
const PATROL_FIELDS: [RecordField; 2] = [CHANNEL_FIELDS[0], CHANNEL_FIELDS[1]];
const SCENARIO_FIELDS: [RecordField; 7] = [
    CHANNEL_FIELDS[2],
    CHANNEL_FIELDS[3],
    CHANNEL_FIELDS[4],
    CHANNEL_FIELDS[5],
    CHANNEL_FIELDS[6],
    CHANNEL_FIELDS[7],
    CHANNEL_FIELDS[8],
];
pub const SCENARIO_REFUSALS: [&str; 7] = [
    "air history scenario channel 10 unreadable",
    "air history scenario channel 17 unreadable",
    "air history scenario channel 51 unreadable",
    "air history scenario channel 58 unreadable",
    "air history scenario channel 44 unreadable",
    "air history scenario channel 24 unreadable",
    "air history scenario channel 65 unreadable",
];
const PATROL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PATROL_FIELDS, 72).expect("valid layout");
const SCENARIO_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SCENARIO_FIELDS, 72).expect("valid layout");

fn qword(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn address(bytes: &[u8], at: usize) -> Result<usize, &'static str> {
    usize::try_from(qword(bytes, at)).map_err(|_| "naval-patrol pointer outside address space")
}

fn header(bytes: &[u8], at: usize) -> Result<Array, &'static str> {
    Array::from_header(bytes[at..at + 16].try_into().unwrap(), 8)
        .ok_or("naval-patrol vector header invalid")
}

fn rva(base: usize, key: Rva) -> Result<u64, &'static str> {
    engine_sites::at(base, key)
        .map(|at| at as u64)
        .ok_or("module address overflow")
}

fn gather(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    fields: usize,
    output: &mut [u8],
    reason: &'static str,
) -> Result<(), &'static str> {
    if memory.copy_records(RecordSources::Pointers(pointers), layout, output)
        != pointers.len() * fields
    {
        return Err(reason);
    }
    Ok(())
}

fn pointers(
    memory: &impl Memory,
    vector: Array,
    first: u32,
    output: &mut [usize; BATCH],
) -> Result<usize, &'static str> {
    let count = ((vector.count - first) as usize).min(BATCH);
    let mut bytes = [0_u8; BATCH * 8];
    let start = vector
        .element(first, 8)
        .ok_or("naval-patrol vector address overflow")?;
    if !read_slice(memory, start, &mut bytes[..count * 8]) {
        return Err("naval-patrol vector unreadable");
    }
    for (index, pointer) in output[..count].iter_mut().enumerate() {
        *pointer = address(&bytes, index * 8)?;
        if *pointer == 0 {
            return Err("naval-patrol vector holds a null entry");
        }
    }
    Ok(count)
}

pub fn task_force_combats(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    mut emit: impl FnMut(TaskForceCombat) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let fleet_vtable = rva(base, cfleet::VTABLE_RVA)?;
    let task_vtables = [
        rva(base, ctask_force::VTABLE_RVA)?,
        rva(base, ctask_force::SECONDARY_VTABLE_RVA)?,
    ];
    let vtables = [
        rva(base, cnaval_combat::VTABLE_RVA)?,
        rva(base, cprovince::VTABLE_RVA)?,
        rva(base, cstrategic_region::VTABLE_RVA)?,
    ];
    let fleets = country
        .pointer
        .checked_add(engine_native::ccountry::FLEETS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("naval-patrol fleet vector unreadable or invalid")?;
    let (mut fleet_pointers, mut task_pointers) = ([0; BATCH], [0; BATCH]);
    let mut fleet_bytes = [0_u8; BATCH * 24];
    let mut task_bytes = [0_u8; BATCH * 32];
    let mut first_fleet = 0;
    while first_fleet < fleets.count {
        let count = pointers(memory, fleets, first_fleet, &mut fleet_pointers)?;
        first_fleet += count as u32;
        gather(
            memory,
            &fleet_pointers[..count],
            &FLEET_LAYOUT,
            FLEET_FIELDS.len(),
            &mut fleet_bytes[..count * 24],
            "naval-patrol fleet unreadable",
        )?;
        for fleet in fleet_bytes[..count * 24].chunks_exact(24) {
            if qword(fleet, 0) != fleet_vtable {
                return Err("naval-patrol fleet vtable mismatch");
            }
            let tasks = header(fleet, 8)?;
            let mut first_task = 0;
            while first_task < tasks.count {
                let count = pointers(memory, tasks, first_task, &mut task_pointers)?;
                first_task += count as u32;
                gather(
                    memory,
                    &task_pointers[..count],
                    &TASK_LAYOUT,
                    TASK_FIELDS.len(),
                    &mut task_bytes[..count * 32],
                    "naval-patrol task force unreadable",
                )?;
                for task in task_bytes[..count * 32].chunks_exact(32) {
                    if [qword(task, 0), qword(task, 8)] != task_vtables {
                        return Err("naval-patrol task force vtable mismatch");
                    }
                    let combats = header(task, 16)?;
                    if combats.count == 0 {
                        continue;
                    }
                    if let Some(combat) = first_naval_combat(memory, combats, vtables)? {
                        emit(combat)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn first_naval_combat(
    memory: &impl Memory,
    combats: Array,
    [naval, province_vtable, region_vtable]: [u64; 3],
) -> Result<Option<TaskForceCombat>, &'static str> {
    let unreadable = "naval-patrol combat unreadable";
    let combat = combats
        .element(0, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or(unreadable)?;
    let mut bytes = [0_u8; 20];
    let fields = memory.copy_records(
        RecordSources::Pointers(&[combat]),
        &COMBAT_LAYOUT,
        &mut bytes[..16],
    );
    if fields == 0 {
        return Err(unreadable);
    }
    if qword(&bytes, 0) != naval {
        return Ok(None);
    }
    if fields != COMBAT_FIELDS.len() {
        return Err(unreadable);
    }
    let province = address(&bytes, 8)?;
    gather(
        memory,
        &[province],
        &PROVINCE_LAYOUT,
        3,
        &mut bytes,
        "naval-patrol combat province unreadable",
    )?;
    if qword(&bytes, 0) != province_vtable {
        return Err("naval-patrol combat province vtable mismatch");
    }
    let id = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let region = address(&bytes, 12)?;
    let region = if region == 0 {
        0
    } else {
        gather(
            memory,
            &[region],
            &REGION_LAYOUT,
            2,
            &mut bytes[..12],
            "naval-patrol combat region unreadable",
        )?;
        if qword(&bytes, 0) != region_vtable {
            return Err("naval-patrol combat region vtable mismatch");
        }
        u32::from_le_bytes(bytes[8..12].try_into().unwrap())
    };
    Ok(Some(TaskForceCombat {
        province: id,
        region,
    }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub history: usize,
    pub capacity: u32,
    pub current: u32,
    pub wrapped: bool,
}

impl WindowState {
    pub fn order(self) -> impl Iterator<Item = (u32, usize)> {
        let filled = if self.wrapped {
            self.capacity
        } else {
            self.current
        };
        let start = if self.wrapped { self.current } else { 0 };
        (0..filled).map(move |index| (index, ((start + index) % self.capacity) as usize))
    }
}

const SLOT_CHUNK: usize = 512;

pub fn region_slots(
    memory: &impl Memory,
    roots: &[usize],
    mut each: impl FnMut(usize, u32, usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    const TABLE: [RecordField; 1] = [field(cstrategic_air::HISTORY_TABLE, 0, 16)];
    const TABLE_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&TABLE, 16).expect("valid layout");
    let mut table_bytes = [0_u8; BATCH * 16];
    let mut slot_bytes = [0_u8; SLOT_CHUNK * 8];
    for (first, chunk) in (0..).step_by(BATCH).zip(roots.chunks(BATCH)) {
        gather(
            memory,
            chunk,
            &TABLE_LAYOUT,
            1,
            &mut table_bytes[..chunk.len() * 16],
            "air history table unreadable",
        )?;
        for index in 0..chunk.len() {
            let table = header(&table_bytes, index * 16)?;
            for start in (0..table.count).step_by(SLOT_CHUNK) {
                let len = ((table.count - start) as usize).min(SLOT_CHUNK);
                let at = table.element(start, 8).ok_or("air history slot overflow")?;
                if !read_slice(memory, at, &mut slot_bytes[..len * 8]) {
                    return Err("air history slot unreadable");
                }
                for slot in 0..len {
                    let history = address(&slot_bytes, slot * 8)?;
                    if history != 0 {
                        each(first + index, start + slot as u32, history)?;
                    }
                }
            }
        }
    }
    Ok(())
}

const WINDOW_BATCH: usize = 16;
#[derive(Default)]
pub struct WindowScratch {
    record_bytes: Vec<u8>,
    records: Vec<usize>,
    value_bytes: Vec<u8>,
    values: Vec<usize>,
    channel_bytes: Vec<u8>,
    channels: Vec<[i64; 9]>,
    scenario_failures: Vec<Option<&'static str>>,
}

impl WindowScratch {
    pub const fn new() -> Self {
        Self {
            record_bytes: Vec::new(),
            records: Vec::new(),
            value_bytes: Vec::new(),
            values: Vec::new(),
            channel_bytes: Vec::new(),
            channels: Vec::new(),
            scenario_failures: Vec::new(),
        }
    }

    pub fn size(&mut self, records: usize) -> std::io::Result<()> {
        let pointers = crate::allocation::mul(records, 8)?;
        let channels = crate::allocation::mul(records, 72)?;
        crate::allocation::resize(&mut self.record_bytes, pointers, 0)?;
        crate::allocation::resize(&mut self.records, records, 0)?;
        crate::allocation::resize(&mut self.value_bytes, pointers, 0)?;
        crate::allocation::resize(&mut self.values, records, 0)?;
        crate::allocation::resize(&mut self.channel_bytes, channels, 0)?;
        crate::allocation::resize(&mut self.channels, records, [0; 9])?;
        crate::allocation::resize(&mut self.scenario_failures, records, None)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.record_bytes.capacity()
            + self.records.capacity() * size_of::<usize>()
            + self.value_bytes.capacity()
            + self.values.capacity() * size_of::<usize>()
            + self.channel_bytes.capacity()
            + self.channels.capacity() * size_of::<[i64; 9]>()
            + self.scenario_failures.capacity() * size_of::<Option<&'static str>>()
    }
}

pub fn history_windows(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut WindowScratch,
    mut each: impl FnMut(
        usize,
        WindowState,
        &[[i64; 9]],
        &[Option<&'static str>],
    ) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    read_windows(
        memory,
        base,
        histories,
        scratch,
        |at, state, values, failures| each(at, state, values, failures),
    )
}

fn read_windows(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut WindowScratch,
    mut each: impl FnMut(
        usize,
        WindowState,
        &[[i64; 9]],
        &[Option<&'static str>],
    ) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if histories.is_empty() {
        return Ok(());
    }
    let mut states = [WindowState::default(); BATCH];
    let mut vectors = [0_usize; BATCH];
    for (first, chunk) in (0..)
        .step_by(WINDOW_BATCH)
        .zip(histories.chunks(WINDOW_BATCH))
    {
        let len = chunk.len();
        window_states(memory, base, chunk, &mut states, &mut vectors)?;
        let total = states[..len]
            .iter()
            .try_fold(0usize, |count, state| {
                count.checked_add(state.capacity as usize)
            })
            .ok_or("air history record count overflow")?;
        scratch
            .size(total)
            .map_err(|_| "air history allocation failed")?;
        let capacity = states[0].capacity as usize;
        if states[..len]
            .iter()
            .all(|state| state.capacity as usize == capacity)
        {
            let field = [field(0, 0, capacity * 8)];
            let layout =
                RecordLayout::new(&field, capacity * 8).ok_or("air history records invalid")?;
            gather(
                memory,
                &vectors[..len],
                &layout,
                1,
                &mut scratch.record_bytes[..len * capacity * 8],
                "air history records unreadable",
            )?;
        } else {
            let mut offset = 0;
            for (state, &vector) in states.iter().zip(&vectors).take(len) {
                let end = offset + state.capacity as usize * 8;
                if !read_slice(memory, vector, &mut scratch.record_bytes[offset..end]) {
                    return Err("air history records unreadable");
                }
                offset = end;
            }
        }
        for (index, record) in scratch.records[..total].iter_mut().enumerate() {
            *record = address(&scratch.record_bytes, index * 8)?;
        }
        gather(
            memory,
            &scratch.records[..total],
            &QWORD_LAYOUT,
            1,
            &mut scratch.value_bytes[..total * 8],
            "air history record unreadable",
        )?;
        for (index, value) in scratch.values[..total].iter_mut().enumerate() {
            *value = address(&scratch.value_bytes, index * 8)?;
        }
        gather(
            memory,
            &scratch.values[..total],
            &PATROL_LAYOUT,
            PATROL_FIELDS.len(),
            &mut scratch.channel_bytes[..total * 72],
            "air history channel unreadable",
        )?;
        scratch.scenario_failures[..total].fill(None);
        let mut first_record = 0;
        while first_record < total {
            let completed = memory.copy_records(
                RecordSources::Pointers(&scratch.values[first_record..total]),
                &SCENARIO_LAYOUT,
                &mut scratch.channel_bytes[first_record * 72..total * 72],
            );
            let records = completed / SCENARIO_FIELDS.len();
            first_record += records;
            if first_record < total {
                scratch.scenario_failures[first_record] =
                    Some(SCENARIO_REFUSALS[completed % SCENARIO_FIELDS.len()]);
                scratch.channel_bytes[first_record * 72 + 16..(first_record + 1) * 72].fill(0);
                first_record += 1;
            }
        }
        for (channel, bytes) in scratch.channels[..total]
            .iter_mut()
            .zip(scratch.channel_bytes.chunks_exact(72))
        {
            *channel = std::array::from_fn(|index| qword(bytes, index * 8) as i64);
        }
        let mut offset = 0;
        for (index, state) in states[..len].iter().enumerate() {
            let capacity = state.capacity as usize;
            each(
                first + index,
                *state,
                &scratch.channels[offset..offset + capacity],
                &scratch.scenario_failures[offset..offset + capacity],
            )?;
            offset += capacity;
        }
    }
    Ok(())
}

fn window_states(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    states: &mut [WindowState; BATCH],
    vectors: &mut [usize; BATCH],
) -> Result<(), &'static str> {
    if histories.is_empty() {
        return Ok(());
    }
    let history_vtable = rva(base, cloop_history::VTABLE_RVA)?;
    let window_vtable = rva(base, loop_history_window::VTABLE_RVA)?;
    let mut windows = [0_usize; BATCH];
    let mut history_bytes = [0_u8; BATCH * 20];
    let mut window_bytes = [0_u8; BATCH * 33];
    let live = histories.len();
    gather(
        memory,
        histories,
        &HISTORY_LAYOUT,
        HISTORY_FIELDS.len(),
        &mut history_bytes[..live * 20],
        "air history unreadable",
    )?;
    for (window, history) in windows
        .iter_mut()
        .zip(history_bytes.chunks_exact(20))
        .take(live)
    {
        if qword(history, 0) != history_vtable
            || u32::from_le_bytes(history[8..12].try_into().unwrap())
                != cloop_history::air_stats::FIELD_COUNT
        {
            return Err("air history vtable or field count mismatch");
        }
        *window = address(history, 12)?;
        if *window == 0 {
            return Err("air history window is null");
        }
    }
    gather(
        memory,
        &windows[..live],
        &WINDOW_LAYOUT,
        WINDOW_FIELDS.len(),
        &mut window_bytes[..live * 33],
        "air history window unreadable",
    )?;
    for (index, window) in window_bytes.chunks_exact(33).take(live).enumerate() {
        if qword(window, 0) != window_vtable {
            return Err("air history window vtable mismatch");
        }
        let vector = header(window, 8)?;
        let word = |at: usize| i32::from_le_bytes(window[at..at + 4].try_into().unwrap());
        let (capacity, current) = (word(24), word(28));
        if capacity <= 0 || capacity as u32 > vector.count || !(0..capacity).contains(&current) {
            return Err("air history window invalid");
        }
        states[index] = WindowState {
            history: histories[index],
            capacity: capacity as u32,
            current: current as u32,
            wrapped: window[32] != 0,
        };
        vectors[index] = vector.element(0, 8).ok_or("air history records overflow")?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "air_kamikaze/sizing_tests.rs"]
mod sizing_tests;

pub fn country_air_roots(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    first: usize,
    step: usize,
    output: &mut Vec<usize>,
) -> Result<usize, &'static str> {
    const MANAGER: [RecordField; 2] = [
        field(0, 0, 8),
        field(engine_native_air::cstrategic_air_manager::COUNTRY_AIR, 8, 16),
    ];
    const ROOTS: usize = 512;
    let manager = game_state
        .checked_add(engine_native::cgame_state::STRATEGIC_AIR_MANAGER)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("strategic air manager unreadable")?;
    let mut bytes = [0_u8; 24];
    gather(
        memory,
        &[manager],
        &RecordLayout::new(&MANAGER, 24).expect("valid layout"),
        2,
        &mut bytes,
        "strategic air manager unreadable",
    )?;
    if qword(&bytes, 0) != rva(base, engine_native_air::cstrategic_air_manager::VTABLE_RVA)? {
        return Err("strategic air manager vtable mismatch");
    }
    let slots = header(&bytes, 8)?;
    output.clear();
    let owned = (slots.count as usize).saturating_sub(first).div_ceil(step);
    crate::allocation::reserve_push_batch(output, owned)
        .map_err(|_| "country air allocation failed")?;
    let stride = step.checked_mul(8).ok_or("country air slot overflow")?;
    let mut chunk = [0_u8; ROOTS * 8];
    for start in (0..owned).step_by(ROOTS) {
        let len = (owned - start).min(ROOTS);
        let at = u32::try_from(first + start * step)
            .ok()
            .and_then(|slot| slots.element(slot, 8))
            .ok_or("country air slot overflow")?;
        let sources = RecordSources::Strided {
            base: at,
            stride,
            count: len,
        };
        if memory.copy_records(sources, &QWORD_LAYOUT, &mut chunk[..len * 8]) != len {
            return Err("country air slots unreadable");
        }
        for index in 0..len {
            output.push(address(&chunk, index * 8)?);
        }
    }
    Ok(slots.count as usize)
}

pub fn authenticate_roots(
    memory: &impl Memory,
    base: usize,
    countries: &engine_native::ccountry::Reader<'_, impl Memory>,
    roots: &[usize],
    slot: impl Fn(usize) -> u32,
) -> Result<(), &'static str> {
    const ROOT: [RecordField; 2] = [field(0, 0, 8), field(cstrategic_air::COUNTRY, 8, 4)];
    const ROOT_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ROOT, 12).expect("valid layout");
    let vtable = rva(base, cstrategic_air::VTABLE_RVA)?;
    let mut bytes = [0_u8; BATCH * 12];
    for (first, chunk) in (0..).step_by(BATCH).zip(roots.chunks(BATCH)) {
        let fields = memory.copy_records(
            RecordSources::Pointers(chunk),
            &ROOT_LAYOUT,
            &mut bytes[..chunk.len() * 12],
        );
        for (index, root) in bytes.chunks_exact(12).take(fields / 2).enumerate() {
            let country = u32::from_le_bytes(root[8..12].try_into().unwrap());
            if qword(root, 0) != vtable
                || countries.resolve(country)?.canonical_slot != slot(first + index)
            {
                return Err("strategic air country vtable or owner mismatch");
            }
        }
        if fields != chunk.len() * 2 {
            return Err("strategic air country unreadable");
        }
    }
    Ok(())
}

pub fn window_sum(values: &[i64]) -> i64 {
    values.iter().fold(0_i64, |sum, &value| {
        sum.wrapping_add(value).clamp(-SUM_LIMIT, SUM_LIMIT)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::{country_identity::Country, memory::SliceMemory};
    use std::cell::Cell;

    const BASE: usize = 0x1_0000_0000;
    const COUNTRY: usize = 0x1000;
    const FLEETS: usize = 0x2000;
    const FLEET: usize = 0x3000;
    const TASKS: usize = 0x10_0000;
    const TASK: usize = 0x20_0000;
    const COMBAT: usize = 0x80_0000;
    const PROVINCE: usize = 0x90_0000;
    const REGION: usize = 0xa0_0000;
    const FLEET_SIZE: usize = 0x100;
    const TASK_SIZE: usize = 0x200;

    struct Image(Vec<u8>);

    impl Image {
        fn q(&mut self, at: usize, value: u64) {
            self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn d(&mut self, at: usize, value: u32) {
            self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn vector(&mut self, at: usize, data: usize, count: u32) {
            self.q(at, data as u64);
            self.d(at + 8, count);
            self.d(at + 12, count);
        }
    }

    struct Counting<'a>(SliceMemory<'a>, Cell<usize>);

    impl Memory for Counting<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.1.set(self.1.get() + 1);
            self.0.copy(address, destination)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.1.set(self.1.get() + 1);
            self.0.copy_records(sources, layout, destination)
        }
    }

    fn fleets(
        fleets: usize,
        tasks: usize,
        combat: impl Fn(usize, usize) -> Option<(bool, u32, u32)>,
    ) -> Image {
        let mut image = Image(vec![0; 0xb0_0000]);
        image.vector(COUNTRY + engine_native::ccountry::FLEETS, FLEETS, fleets as u32);
        for f in 0..fleets {
            let fleet = FLEET + f * FLEET_SIZE;
            image.q(FLEETS + f * 8, fleet as u64);
            image.q(fleet, (BASE + cfleet::VTABLE_RVA.0) as u64);
            let list = TASKS + f * tasks * 8;
            image.vector(fleet + cfleet::TASK_FORCES, list, tasks as u32);
            for t in 0..tasks {
                let index = f * tasks + t;
                let task = TASK + index * TASK_SIZE;
                image.q(list + t * 8, task as u64);
                image.q(task, (BASE + ctask_force::VTABLE_RVA.0) as u64);
                image.q(
                    task + ctask_force::SECONDARY_BASE,
                    (BASE + ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
                );
                let Some((naval, province, region)) = combat(f, t) else {
                    continue;
                };
                let combat = COMBAT + index * 0x40;
                let province_at = PROVINCE + index * 0x100;
                let region_at = REGION + index * 0x80;
                image.vector(task + cunit::COMBAT_REFERENCE_DATA, task + 0x1c0, 1);
                image.q(task + 0x1c0, combat as u64);
                let vtable = if naval {
                    cnaval_combat::VTABLE_RVA
                } else {
                    engine_native::cland_combat::VTABLE_RVA
                };
                image.q(combat, (BASE + vtable.0) as u64);
                image.q(combat + cnaval_combat::PROVINCE, province_at as u64);
                image.q(province_at, (BASE + cprovince::VTABLE_RVA.0) as u64);
                image.d(province_at + cprovince::ID, province);
                if region != 0 {
                    image.q(province_at + cprovince::STRATEGIC_REGION, region_at as u64);
                    image.q(region_at, (BASE + cstrategic_region::VTABLE_RVA.0) as u64);
                    image.d(region_at + cstrategic_region::ID, region);
                }
            }
        }
        image
    }

    fn walk(image: &Image) -> (Result<Vec<TaskForceCombat>, &'static str>, usize) {
        let memory = Counting(
            SliceMemory {
                base: 0,
                bytes: &image.0,
            },
            Cell::new(0),
        );
        let country = Country {
            pointer: COUNTRY,
            canonical_slot: 1,
            id: 1,
        };
        let mut out = Vec::new();
        let result = task_force_combats(&memory, BASE, country, |combat| {
            out.push(combat);
            Ok(())
        })
        .map(|()| out);
        (result, memory.1.get())
    }

    #[test]
    fn task_forces_in_naval_combat_follow_fleet_order_and_skip_land_combats() {
        let image = fleets(2, 2, |f, t| match (f, t) {
            (0, 0) => Some((false, 5, 7)),
            (0, 1) => Some((true, 11, 7)),
            (1, 1) => Some((true, 12, 0)),
            _ => None,
        });
        let (combats, calls) = walk(&image);
        assert_eq!(
            combats.unwrap(),
            [
                TaskForceCombat {
                    province: 11,
                    region: 7
                },
                TaskForceCombat {
                    province: 12,
                    region: 0
                },
            ]
        );
        assert!(calls <= 3 + 2 * 2 + 3 * 4, "{calls} calls");
    }

    #[test]
    fn a_land_combat_is_skipped_where_a_naval_combats_province_would_be_unreadable() {
        let mut image = fleets(1, 2, |_, t| Some((t == 1, 11, 7)));
        let land = image.0.len() - 8;
        image.q(land, (BASE + engine_native::cland_combat::VTABLE_RVA.0) as u64);
        image.q(TASK + 0x1c0, land as u64);
        assert_eq!(
            walk(&image).0,
            Ok(vec![TaskForceCombat {
                province: 11,
                region: 7
            }])
        );
        image.q(land, (BASE + cnaval_combat::VTABLE_RVA.0) as u64);
        assert_eq!(walk(&image).0, Err("naval-patrol combat unreadable"));
    }

    #[test]
    fn a_foreign_task_force_fleet_or_province_vtable_is_refused() {
        let mut image = fleets(1, 1, |_, _| None);
        image.q(TASK + ctask_force::SECONDARY_BASE, 0);
        assert_eq!(
            walk(&image).0,
            Err("naval-patrol task force vtable mismatch")
        );
        let mut image = fleets(1, 1, |_, _| None);
        image.q(FLEET, 0);
        assert_eq!(walk(&image).0, Err("naval-patrol fleet vtable mismatch"));
        let mut image = fleets(1, 1, |_, _| Some((true, 4, 2)));
        image.q(PROVINCE, 0);
        assert_eq!(
            walk(&image).0,
            Err("naval-patrol combat province vtable mismatch")
        );
    }

    fn air_objects(countries: u32) -> (engine_native::ccountry::synthetic::World, Vec<usize>) {
        let mut world = engine_native::ccountry::synthetic::World::new(BASE, countries, 0x10);
        let roots = (0..=countries)
            .map(|slot| {
                let root = world.alloc(cstrategic_air::COUNTRY + 4);
                world.u64(root, (BASE + cstrategic_air::VTABLE_RVA.0) as u64);
                world.u32(root + cstrategic_air::COUNTRY, slot);
                root
            })
            .collect();
        (world, roots)
    }

    #[test]
    fn air_objects_are_authenticated_64_to_a_gather_in_slot_order() {
        let (mut world, roots) = air_objects(200);
        let slots: Vec<u32> = (1..=200).collect();
        let targets: Vec<usize> = slots.iter().map(|&slot| roots[slot as usize]).collect();
        let check = |world: &engine_native::ccountry::synthetic::World, targets: &[usize]| {
            let countries = world.memory();
            let readers =
                engine_native::ccountry::Reader::new(&countries, BASE, world.game_state()).unwrap();
            let memory = Counting(world.memory(), Cell::new(0));
            let result = authenticate_roots(&memory, BASE, &readers, targets, |at| slots[at]);
            (result, memory.1.get())
        };
        let (result, calls) = check(&world, &targets);
        assert_eq!(result, Ok(()));
        assert_eq!(calls, 200_usize.div_ceil(64));
        world.u32(roots[70] + cstrategic_air::COUNTRY, 71);
        assert_eq!(
            check(&world, &targets).0,
            Err("strategic air country vtable or owner mismatch")
        );
        let mut faulty = targets.clone();
        faulty[75] = 0;
        assert_eq!(
            check(&world, &faulty).0,
            Err("strategic air country vtable or owner mismatch")
        );
        faulty[65] = 0;
        assert_eq!(
            check(&world, &faulty).0,
            Err("strategic air country unreadable")
        );
    }

    #[test]
    fn a_window_sums_from_its_current_record_once_wrapped() {
        let state = |current, wrapped| WindowState {
            history: 1,
            capacity: 4,
            current,
            wrapped,
        };
        let order = |state: WindowState| state.order().collect::<Vec<_>>();
        assert_eq!(order(state(2, false)), [(0, 0), (1, 1)]);
        assert_eq!(order(state(2, true)), [(0, 2), (1, 3), (2, 0), (3, 1)]);
    }

    #[test]
    fn the_window_sum_clamps_after_each_record() {
        assert_eq!(window_sum(&[]), 0);
        assert_eq!(window_sum(&[3, -1]), 2);
        assert_eq!(window_sum(&[SUM_LIMIT, 5, -5]), SUM_LIMIT - 5);
        assert_eq!(window_sum(&[-SUM_LIMIT, -1]), -SUM_LIMIT);
    }

    #[test]
    fn scenario_windows_capture_all_raw_channels_and_preserve_native_record_order() {
        const HISTORY: usize = 0x100;
        const WINDOW: usize = 0x200;
        const RECORDS: usize = 0x300;
        const RECORD: usize = 0x400;
        const VALUES: usize = 0x1000;
        let mut image = Image(vec![0; 0x3000]);
        image.q(HISTORY, (BASE + cloop_history::VTABLE_RVA.0) as u64);
        image.d(
            HISTORY + cloop_history::FIELD_COUNT,
            cloop_history::air_stats::FIELD_COUNT,
        );
        image.q(HISTORY + FIRST_WINDOW, WINDOW as u64);
        image.q(WINDOW, (BASE + loop_history_window::VTABLE_RVA.0) as u64);
        image.vector(WINDOW + loop_history_window::RECORDS, RECORDS, 4);
        image.d(WINDOW + loop_history_window::CAPACITY, 4);
        image.d(WINDOW + loop_history_window::CURRENT_SLOT, 2);
        image.0[WINDOW + loop_history_window::WRAPPED] = 1;
        let expected: Vec<[i64; 9]> = (0..4)
            .map(|record| std::array::from_fn(|channel| (record * 100 + channel) as i64 - 3))
            .collect();
        for (record, values) in expected.iter().enumerate() {
            image.q(RECORDS + record * 8, (RECORD + record * 8) as u64);
            image.q(RECORD + record * 8, (VALUES + record * 0x300) as u64);
            for (&channel, &value) in CHANNELS.iter().zip(values) {
                image.q(
                    VALUES + record * 0x300 + channel * loop_history_record_fields::VALUE_STRIDE,
                    value as u64,
                );
            }
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &image.0,
        };
        let mut scratch = WindowScratch::new();
        let mut seen = Vec::new();
        history_windows(
            &memory,
            BASE,
            &[HISTORY],
            &mut scratch,
            |index, state, values, failures| {
                assert!(failures.iter().all(Option::is_none));
                assert_eq!(index, 0);
                assert_eq!(values, expected);
                seen.extend(state.order().map(|(_, record)| values[record]));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, [expected[2], expected[3], expected[0], expected[1]]);

        struct Fault<'a> {
            bytes: &'a [u8],
            address: usize,
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, destination.len()));
                if address <= self.address
                    && self.address < address.saturating_add(destination.len())
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
        for (suffix, &channel) in CHANNELS[2..].iter().enumerate() {
            let fault = Fault {
                bytes: &image.0,
                address: VALUES + 0x300 + channel * loop_history_record_fields::VALUE_STRIDE,
                reads: Default::default(),
            };
            let mut callbacks = 0;
            history_windows(
                &fault,
                BASE,
                &[HISTORY],
                &mut scratch,
                |_, _, values, failures| {
                    callbacks += 1;
                    for record in 0..4 {
                        assert_eq!(&values[record][..2], &expected[record][..2]);
                        if record == 1 {
                            assert_eq!(failures[record], Some(SCENARIO_REFUSALS[suffix]));
                            assert_eq!(values[record][2..], [0; 7]);
                        } else {
                            assert_eq!(failures[record], None);
                            assert_eq!(values[record], expected[record]);
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(
                callbacks, 1,
                "scenario faults retain the naval-patrol consumer"
            );
            for (index, &channel) in CHANNELS.iter().enumerate() {
                let address = VALUES + 0x300 + channel * loop_history_record_fields::VALUE_STRIDE;
                let attempts = fault
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == address)
                    .count();
                assert_eq!(attempts, usize::from(index <= suffix + 2));
            }
        }
    }
}
