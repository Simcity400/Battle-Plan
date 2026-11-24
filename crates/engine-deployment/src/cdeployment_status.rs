
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::ccountry;
use engine_native_parts::{cequipment_type, csub_unit_definition, equipment_entry};
use crate::{
    cmilitary_deployment,
    cmilitary_deployment_conveyor,
    cmilitary_deployment_line,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29dc598);
pub const CONVEYORS: usize = 0x48;
pub const CONVEYOR_STRIDE: usize = 0x08;
pub const OWNER: usize = 0x118;
pub const UNIT_BONUSES: usize = 0x08;

pub const SELECTED_STAFF_TEMPLATE: usize = 0xac;
pub const NEXT_HQ_ORDER: usize = 0xa8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TemplateState {
    pub country: u32,
    pub selected_staff: u64,
    pub next_hq_order: i32,
    pub lock_all: u8,
    pub manager: usize,
}

pub fn template_state(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<TemplateState, &'static str> {
    let at = manager(memory, base, country)?;
    let selected_staff = at
        .checked_add(SELECTED_STAFF_TEMPLATE)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("military staff template unreadable")?;
    let next_hq_order = at
        .checked_add(NEXT_HQ_ORDER)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("headquarters deployment order unreadable")?;
    let lock_all = country
        .pointer
        .checked_add(ccountry::LOCK_ALL_TEMPLATES)
        .and_then(|at| read::<u8>(memory, at))
        .ok_or("country template lock unreadable")?;
    Ok(TemplateState {
        country: country.id,
        selected_staff,
        next_hq_order,
        lock_all,
        manager: at,
    })
}

pub use cmilitary_deployment_conveyor::Row as ConveyorRow;
pub use cmilitary_deployment_line::Row as LineRow;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManpowerRow {
    pub line: u64,
    pub side: u8,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentRow {
    pub line: u64,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub conveyors: Outcome,
    pub lines: Outcome,
    pub manpower: Outcome,
    pub equipment: Outcome,
    pub name_bytes: usize,
    pub required_name_bytes: usize,
    pub custom_name_bytes: usize,
    pub required_custom_name_bytes: usize,
}

impl Outcomes {
    pub fn unavailable(reason: &'static str) -> Self {
        Self {
            conveyors: Outcome::Unavailable { reason },
            lines: Outcome::Unavailable { reason },
            manpower: Outcome::Unavailable { reason },
            equipment: Outcome::Unavailable { reason },
            name_bytes: 0,
            required_name_bytes: 0,
            custom_name_bytes: 0,
            required_custom_name_bytes: 0,
        }
    }
}

pub(crate) fn manager(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::DEPLOYMENT_MANAGER)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|pointer| *pointer != 0)
        .ok_or("deployment status pointer unreadable or null")?;
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("deployment status module address overflow")?;
    let mut vtable = [0; 8];
    let mut owner = [0; 8];
    let copied = memory.copy_scattered(
        [pointer, pointer.checked_add(OWNER).unwrap_or(0)],
        &mut [&mut vtable, &mut owner],
    );
    if copied == 0 || u64::from_le_bytes(vtable) != expected as u64 {
        return Err("deployment status vtable mismatch");
    }
    if copied < 2 || u64::from_le_bytes(owner) != country.pointer as u64 {
        return Err("deployment status country mismatch");
    }
    Ok(pointer)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the independent deployment rows and literal arenas share one original graph"
)]
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    conveyors: &mut Vec<ConveyorRow>,
    lines: &mut Vec<LineRow>,
    manpower: &mut Vec<ManpowerRow>,
    equipment: &mut Vec<EquipmentRow>,
    names: &mut Vec<u8>,
    custom_names: &mut Vec<u8>,
) -> Outcomes {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(conveyors),
        engine_native_memory::walk::Buffer::Growing(lines),
        engine_native_memory::walk::Buffer::Growing(manpower),
        engine_native_memory::walk::Buffer::Growing(equipment),
        engine_native_memory::walk::Buffer::Growing(names),
        engine_native_memory::walk::Buffer::Growing(custom_names),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the independent deployment rows and literal arenas share one original graph"
)]
pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    conveyors: engine_native_memory::walk::Buffer<'_, ConveyorRow>,
    lines: engine_native_memory::walk::Buffer<'_, LineRow>,
    manpower: engine_native_memory::walk::Buffer<'_, ManpowerRow>,
    equipment: engine_native_memory::walk::Buffer<'_, EquipmentRow>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    mut custom_names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Outcomes {
    let mut conveyors = Sink::new(conveyors);
    let mut lines = Sink::new(lines);
    let mut manpower = Sink::new(manpower);
    let mut equipment = Sink::new(equipment);
    let mut used = 0;
    let mut required = 0_usize;
    let mut custom_used = 0;
    let mut custom_required = 0_usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let status = manager(memory, base, country)?;
            let queue = status
                .checked_add(CONVEYORS)
                .and_then(|address| Array::read(memory, address, CONVEYOR_STRIDE))
                .ok_or("deployment conveyor array unreadable or invalid")?;
            conveyors.ensure(queue.count as usize)?;
            for index in 0..queue.count {
                let pointer = queue
                    .element(index, CONVEYOR_STRIDE)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|pointer| *pointer != 0)
                    .ok_or("deployment conveyor pointer unreadable or null")?;
                let conveyor = cmilitary_deployment_conveyor::read_row_buffer(
                    memory, base, country.id, index, pointer, &mut names, used,
                )?;
                required = required.saturating_add(conveyor.wanted);
                match conveyor.row.filter(|_| conveyors.remaining() != 0) {
                    Some(row) => {
                        used = conveyor.used;
                        conveyors.push(row);
                    }
                    None => conveyors.omit(1),
                }
                let queued_lines = conveyor.lines;
                lines.ensure(queued_lines.count as usize)?;
                for ordinal in 0..queued_lines.count {
                    let line = queued_lines
                        .element(ordinal, cmilitary_deployment_conveyor::LINE_STRIDE)
                        .and_then(|address| read::<u64>(memory, address))
                        .and_then(|pointer| usize::try_from(pointer).ok())
                        .filter(|pointer| *pointer != 0)
                        .ok_or("deployment line pointer unreadable or null")?;
                    let read = cmilitary_deployment_line::read_row_buffer(
                        memory,
                        base,
                        conveyor.handle,
                        pointer,
                        country.id,
                        ordinal,
                        line,
                        &mut custom_names,
                        custom_used,
                    )?;
                    custom_required = custom_required.saturating_add(read.wanted);
                    let Some(line) = read.row.filter(|_| lines.remaining() != 0) else {
                        lines.omit(1);
                        manpower.omit(read.children.manpower[0].count);
                        manpower.omit(read.children.manpower[1].count);
                        equipment.omit(read.children.equipment.count);
                        continue;
                    };
                    custom_used = read.used;
                    let children = read.children;
                    lines.push(line);
                    for (side, array) in children.manpower.into_iter().enumerate() {
                        manpower.ensure(array.count as usize)?;
                        let take = manpower.remaining().min(array.count as usize) as u32;
                        for ordinal in 0..take {
                            let (source_country, amount) =
                                cmilitary_deployment::manpower_entry(memory, array, ordinal)?;
                            manpower.push(ManpowerRow {
                                line: line.handle,
                                side: side as u8,
                                ordinal,
                                source_country,
                                amount,
                            });
                        }
                        manpower.omit(array.count - take);
                    }
                    let array = children.equipment;
                    equipment.ensure(array.count as usize)?;
                    let take = equipment.remaining().min(array.count as usize) as u32;
                    for ordinal in 0..take {
                        let entry: equipment_entry::Row =
                            cmilitary_deployment::equipment_entry(memory, base, array, ordinal)?;
                        equipment.push(EquipmentRow {
                            line: line.handle,
                            ordinal,
                            variant_handle: entry.variant_handle,
                            amount_raw: entry.amount_raw,
                        });
                    }
                    equipment.omit(array.count - take);
                }
            }
        }
        if found_country {
            Ok(())
        } else {
            Err("country table has no readable countries")
        }
    })();
    match result {
        Err(reason) => Outcomes::unavailable(reason),
        Ok(()) => Outcomes {
            conveyors: conveyors.finish(),
            lines: lines.finish(),
            manpower: manpower.finish(),
            equipment: equipment.finish(),
            name_bytes: used,
            required_name_bytes: required,
            custom_name_bytes: custom_used,
            required_custom_name_bytes: custom_required,
        },
    }
}

struct Sink<'a, R> {
    rows: engine_native_memory::walk::Buffer<'a, R>,
    len: usize,
    unread_rows: u32,
    overflowed: bool,
}

impl<'a, R: Default> Sink<'a, R> {
    fn new(rows: engine_native_memory::walk::Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread_rows: 0,
            overflowed: false,
        }
    }
    fn remaining(&self) -> usize {
        self.rows.len() - self.len
    }
    fn ensure(&mut self, count: usize) -> Result<(), &'static str> {
        self.rows.ensure(
            self.len
                .checked_add(count)
                .ok_or("deployment row count overflow")?,
        )?;
        Ok(())
    }
    fn push(&mut self, row: R) {
        self.rows[self.len] = row;
        self.len += 1;
    }
    fn omit(&mut self, count: u32) {
        match self.unread_rows.checked_add(count) {
            Some(total) => self.unread_rows = total,
            None => self.overflowed = true,
        }
    }
    fn finish(self) -> Outcome {
        match (self.overflowed, self.unread_rows) {
            (true, _) => Outcome::Unavailable {
                reason: "deployment unread row count exceeds u32",
            },
            (false, 0) => Outcome::Rows { len: self.len },
            (false, unread_rows) => Outcome::Truncated {
                len: self.len,
                unread_rows,
            },
        }
    }
}

pub const LOCAL_SUBUNITS: usize = 0x20;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalSubunitRow {
    pub country: u32,
    pub token: u32,
    pub requirements: engine_native::creferenced_division_template::Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalRequirement {
    pub archetype_token: u32,
    pub amount_raw: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSubunitWalk {
    pub outcome: Outcome,
    pub required_rows: usize,
    pub requirements_len: usize,
    pub required_requirements: usize,
}

const fn record_field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

const CLONE_FIELDS: [RecordField; 3] = [
    record_field(0, 0, 17),
    record_field(csub_unit_definition::DATABASE_INDEX, 20, 4),
    record_field(csub_unit_definition::ARCHETYPE_REQUIREMENTS_DATA, 24, 16),
];
const CLONE_RECORD: usize = 40;
const CLONE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CLONE_FIELDS, CLONE_RECORD).expect("valid local subunit layout");
const CLONE_BATCH: usize = 64;
const REQUIREMENT_SPAN: usize = 16;

fn same_requirements(
    memory: &impl Memory,
    local: Array,
    global: Array,
) -> Result<bool, &'static str> {
    if local.count != global.count {
        return Ok(false);
    }
    const STRIDE: usize = csub_unit_definition::ARCHETYPE_REQUIREMENT_STRIDE;
    let mut own = [0_u8; REQUIREMENT_SPAN * STRIDE];
    let mut theirs = [0_u8; REQUIREMENT_SPAN * STRIDE];
    for first in (0..local.count).step_by(REQUIREMENT_SPAN) {
        let span = ((local.count - first) as usize).min(REQUIREMENT_SPAN) * STRIDE;
        for (vector, bytes) in [(local, &mut own), (global, &mut theirs)] {
            let at = vector
                .element(first, STRIDE)
                .ok_or("subunit requirement index invalid")?;
            if !read_slice(memory, at, &mut bytes[..span]) {
                return Err("subunit requirements unreadable");
            }
        }
        if own[..span] != theirs[..span] {
            return Ok(false);
        }
    }
    Ok(true)
}

fn archetype_token(
    memory: &impl Memory,
    base: usize,
    archetype: usize,
) -> Result<u32, &'static str> {
    let identity = read::<[u8; 17]>(memory, archetype).ok_or("requirement archetype unreadable")?;
    let token = u32::from_le_bytes(
        identity[cequipment_type::TOKEN..cequipment_type::TOKEN + 4]
            .try_into()
            .unwrap(),
    );
    if Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        != engine_sites::at(base, cequipment_type::VTABLE_RVA).map(|vtable| vtable as u64)
        || identity[cequipment_type::ACTIVE] == 0
        || token == 0
    {
        return Err("requirement archetype identity invalid");
    }
    Ok(token)
}

pub fn count_local_subunits(memory: &impl Memory, base: usize, game_state: usize) -> LocalSubunitWalk {
    walk_local_subunits_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, |_| LocalSelection::Read, |_| {})
}

pub fn walk_local_subunits(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<LocalSubunitRow>,
    requirements: &mut Vec<LocalRequirement>,
) -> LocalSubunitWalk {
    walk_local_subunits_selected(
        memory,
        base,
        game_state,
        rows,
        requirements,
        |_| LocalSelection::Read,
        |_| {},
    )
}

#[derive(Clone, Copy, Debug)]
pub struct HeldLocalSubunits<'a> {
    pub rows: &'a [LocalSubunitRow],
    pub requirements: &'a [LocalRequirement],
}

#[derive(Clone, Copy, Debug)]
pub enum LocalSelection<'a> {
    Read,
    Carry(HeldLocalSubunits<'a>),
    Confirm {
        deployment: Deployment,
        held: HeldLocalSubunits<'a>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub status: usize,
    pub clones: Array,
}

#[derive(Clone, Debug)]
pub struct LocalVisit {
    pub country: engine_native_memory::country_identity::Country,
    pub deployment: Option<Deployment>,
    pub rows: std::ops::Range<usize>,
    pub read: bool,
}

pub fn walk_local_subunits_selected<'a>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<LocalSubunitRow>,
    requirements: &mut Vec<LocalRequirement>,
    select: impl FnMut(engine_native_memory::country_identity::Country) -> LocalSelection<'a>,
    visit: impl FnMut(LocalVisit),
) -> LocalSubunitWalk {
    walk_local_subunits_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(requirements),
        select,
        visit,
    )
}

pub fn deployment(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<Deployment, &'static str> {
    let status = manager(memory, base, country)?;
    let clones = status
        .checked_add(LOCAL_SUBUNITS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("local subunit vector invalid")?;
    Ok(Deployment { status, clones })
}

struct LocalOutput<'r> {
    rows: engine_native_memory::walk::Buffer<'r, LocalSubunitRow>,
    requirements: engine_native_memory::walk::Buffer<'r, LocalRequirement>,
    len: usize,
    missing: u32,
}

impl LocalOutput<'_> {
    fn push(
        &mut self,
        result: &mut LocalSubunitWalk,
        mut row: LocalSubunitRow,
        count: usize,
        mut entry: impl FnMut(usize) -> Result<LocalRequirement, &'static str>,
    ) -> Result<(), &'static str> {
        result.required_rows = result
            .required_rows
            .checked_add(1)
            .ok_or("local subunit count overflow")?;
        result.required_requirements = result
            .required_requirements
            .checked_add(count)
            .ok_or("local subunit requirement count overflow")?;
        self.rows.ensure(
            self.len
                .checked_add(1)
                .ok_or("local subunit count overflow")?,
        )?;
        let start = result.requirements_len;
        self.requirements.ensure(
            start
                .checked_add(count)
                .ok_or("local subunit requirement count overflow")?,
        )?;
        if self.len == self.rows.len() || count > self.requirements.len() - start {
            self.missing = self
                .missing
                .checked_add(1)
                .ok_or("local subunit count overflow")?;
            return Ok(());
        }
        for index in 0..count {
            self.requirements[start + index] = entry(index)?;
        }
        result.requirements_len += count;
        row.requirements = engine_native::creferenced_division_template::Span { start, len: count };
        self.rows[self.len] = row;
        self.len += 1;
        Ok(())
    }
}

fn walk_local_subunits_buffer<'a>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: engine_native_memory::walk::Buffer<'_, LocalSubunitRow>,
    requirements: engine_native_memory::walk::Buffer<'_, LocalRequirement>,
    mut select: impl FnMut(engine_native_memory::country_identity::Country) -> LocalSelection<'a>,
    mut visit: impl FnMut(LocalVisit),
) -> LocalSubunitWalk {
    let mut result = LocalSubunitWalk {
        outcome: Outcome::Rows { len: 0 },
        required_rows: 0,
        requirements_len: 0,
        required_requirements: 0,
    };
    let mut output = LocalOutput {
        rows,
        requirements,
        len: 0,
        missing: 0,
    };
    let captured =
        (|| {
            let countries = ccountry::Reader::new(memory, base, game_state)?;
            let mut database = None;
            for slot in 1..countries.count() {
                let Some(country) = countries.at(slot)? else {
                    continue;
                };
                let first = output.len;
                let (current, held) = match select(country) {
                    LocalSelection::Read => (Some(deployment(memory, base, country)?), None),
                    LocalSelection::Carry(held) => (None, Some(held)),
                    LocalSelection::Confirm {
                        deployment: expected,
                        held,
                    } => {
                        let current = deployment(memory, base, country)?;
                        (Some(current), (current == expected).then_some(held))
                    }
                };
                match (held, current) {
                    (Some(held), _) => {
                        for row in held.rows {
                            let entries = row
                                .requirements
                                .start
                                .checked_add(row.requirements.len)
                                .and_then(|end| held.requirements.get(row.requirements.start..end))
                                .ok_or("held local subunit requirements invalid")?;
                            output.push(&mut result, *row, entries.len(), |index| {
                                Ok(entries[index])
                            })?;
                        }
                    }
                    (None, Some(current)) => {
                        let definitions = match database {
                            Some(definitions) => definitions,
                            None => *database
                                .insert(csub_unit_definition::Database::read(memory, base)?),
                        };
                        read_country(
                            memory,
                            base,
                            definitions,
                            country,
                            current.clones,
                            &mut output,
                            &mut result,
                        )?;
                    }
                    (None, None) => unreachable!("only a carried country skips its deployment"),
                }
                visit(LocalVisit {
                    country,
                    deployment: current,
                    rows: first..output.len,
                    read: held.is_none(),
                });
            }
            Ok(())
        })();
    result.outcome = match captured {
        Ok(()) if output.missing == 0 => Outcome::Rows { len: output.len },
        Ok(()) => Outcome::Truncated {
            len: output.len,
            unread_rows: output.missing,
        },
        Err(reason) => Outcome::Unavailable { reason },
    };
    result
}

fn read_country(
    memory: &impl Memory,
    base: usize,
    database: csub_unit_definition::Database,
    country: engine_native_memory::country_identity::Country,
    clones: Array,
    output: &mut LocalOutput<'_>,
    result: &mut LocalSubunitWalk,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
        .ok_or("local subunit module address overflow")? as u64;
    let mut slots = [0_u8; CLONE_BATCH * 8];
    let mut pointers = [0_usize; CLONE_BATCH];
    let mut records = [0_u8; CLONE_BATCH * CLONE_RECORD];
    let country_rows = output.len;
    for first in (0..clones.count).step_by(CLONE_BATCH) {
        let batch = ((clones.count - first) as usize).min(CLONE_BATCH);
        let at = clones
            .element(first, 8)
            .ok_or("local subunit index invalid")?;
        if !read_slice(memory, at, &mut slots[..batch * 8]) {
            return Err("local subunit pointers unreadable");
        }
        for (pointer, bytes) in pointers.iter_mut().zip(slots[..batch * 8].chunks_exact(8)) {
            *pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                .ok()
                .filter(|pointer| *pointer != 0)
                .ok_or("local subunit pointer null")?;
        }
        if memory.copy_records(
            RecordSources::Pointers(&pointers[..batch]),
            &CLONE_LAYOUT,
            &mut records[..batch * CLONE_RECORD],
        ) < batch * CLONE_FIELDS.len()
        {
            return Err("local subunit fields unreadable");
        }
        for (&clone, record) in pointers[..batch]
            .iter()
            .zip(records[..batch * CLONE_RECORD].chunks_exact(CLONE_RECORD))
        {
            let token = u32::from_le_bytes(
                record[csub_unit_definition::TOKEN..csub_unit_definition::TOKEN + 4]
                    .try_into()
                    .unwrap(),
            );
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected
                || record[csub_unit_definition::IS_REAL] != 1
                || token == 0
            {
                return Err("local subunit identity invalid");
            }
            let global = database.entry(
                memory,
                i32::from_le_bytes(record[20..24].try_into().unwrap()),
            )?;
            if global == clone {
                return Err("local subunit is its global definition");
            }
            let own = Array::from_header(
                record[24..40].try_into().unwrap(),
                csub_unit_definition::ARCHETYPE_REQUIREMENT_STRIDE,
            )
            .ok_or("subunit requirement vector invalid")?;
            if same_requirements(
                memory,
                own,
                csub_unit_definition::requirements(memory, global)?,
            )? {
                continue;
            }
            if output.rows[country_rows..output.len]
                .iter()
                .any(|row| row.token == token)
            {
                return Err("local subunit token repeats within a country");
            }
            let row = LocalSubunitRow {
                country: country.id,
                token,
                requirements: Default::default(),
            };
            output.push(result, row, own.count as usize, |entry| {
                let requirement = csub_unit_definition::requirement(memory, own, entry as u32)?;
                Ok(LocalRequirement {
                    archetype_token: archetype_token(memory, base, requirement.archetype)?,
                    amount_raw: requirement.amount,
                })
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod local_subunit_tests;
