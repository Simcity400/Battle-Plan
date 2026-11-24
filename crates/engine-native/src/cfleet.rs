
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordLayout, RecordSources, packed_fields},
    read,
    walk::Outcome,
};
use crate::{ccountry, cgame_state, cprovince, cship, ctask_force};

pub const VTABLE_RVA: Rva = Rva(0x029d_f3d8);
pub const HANDLE: usize = 0x08;
pub const HANDLE_TYPE: u32 = 61;
pub const OWNER: usize = 0xa8;
pub const LEADER: usize = 0xb0;
pub const NAME: usize = 0xe0;
pub const TASK_FORCES: usize = 0xb8;
#[cfg(any(test, feature = "test-support"))]
pub const TASK_FORCE_CAPACITY: usize = 0xc0;
pub const TASK_FORCE_COUNT: usize = 0xc4;
pub const HOME_BASE: usize = 0xd8;
pub const MISSION_REGIONS: usize = 0x48;
pub const MISSION_REGION_COUNT: usize = 0x54;
pub const COLOR: usize = 0x110;
pub const COLOR_COMPONENTS: usize = 0x120;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    pub home_base: u32,
    pub leader: Option<u64>,
    pub name_start: usize,
    pub name_len: usize,
    pub name_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub fleets: Outcome,
    pub task_forces: Outcome,
    pub ships: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedCapture {
    pub capture: Capture,
    pub read_ticks: [u64; 3],
}

struct Output<'a, R> {
    rows: engine_native_memory::walk::Buffer<'a, R>,
    len: usize,
    unread: u32,
    error: Option<&'static str>,
}

impl<'a, R: Default> Output<'a, R> {
    fn from_buffer(rows: engine_native_memory::walk::Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread: 0,
            error: None,
        }
    }

    fn needs_row(&mut self) -> Result<bool, &'static str> {
        if self.error.is_some() {
            return Ok(false);
        }
        if !self
            .rows
            .ensure(self.len.checked_add(1).ok_or("naval row demand overflow")?)?
        {
            self.unread = self
                .unread
                .checked_add(1)
                .ok_or("naval unread row count overflow")?;
            return Ok(false);
        }
        Ok(true)
    }

    fn may_take(&self) -> bool {
        self.error.is_none() && (self.rows.is_growing() || self.len < self.rows.len())
    }

    fn push(&mut self, row: Result<R, &'static str>) {
        match row {
            Ok(row) => {
                self.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.error = Some(reason),
        }
    }

    fn finish(&self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else if self.unread != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        } else {
            Outcome::Rows { len: self.len }
        }
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
) -> Capture {
    walk_timed(memory, base, game_state, fleets, task_forces, ships, |_| 0).capture
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Capture {
    walk_named(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        |_| 0,
        |_, _| {},
        None,
        true,
    )
    .capture
}

pub fn walk_timed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
) -> TimedCapture {
    walk_timed_with(
        memory,
        base,
        game_state,
        fleets,
        task_forces,
        ships,
        now,
        |_, _| {},
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet, task force and ship arenas are caller-owned, and the clock and visit callbacks belong to the caller"
)]
pub fn walk_timed_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &mut cship::Row),
) -> TimedCapture {
    walk_named(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(fleets),
        engine_native_memory::walk::Buffer::Growing(task_forces),
        engine_native_memory::walk::Buffer::Growing(ships),
        now,
        visit,
        None,
        true,
    )
}

const SHIP_HINTS: usize = 8;

const SHIP_RUN: usize = 64;

const FLEET_FIELDS: [(usize, usize); 7] = [
    (0, 8),
    (HANDLE, 8),
    (OWNER, 8),
    (LEADER, 8),
    (TASK_FORCES, 16),
    (HOME_BASE, 8),
    (NAME, engine_native_memory::msvc_string::STRIDE),
];

const TASK_CENSUS_FIELDS: [(usize, usize); 10] = [
    (0, 8),
    (ctask_force::SECONDARY_BASE, 8),
    (crate::cunit::HANDLE, 8),
    (crate::cunit::COMBAT_REFERENCE_COUNT, 4),
    (crate::cunit::ROSTER_OWNER, 12),
    (crate::cunit::LOCATION, 8),
    (crate::cunit::ORDINARY_ROUTE_COUNT, 4),
    (ctask_force::FLEET, 8),
    (ctask_force::SHIPS, 16),
    (ctask_force::MISSION, 4),
];

#[derive(Clone, Copy)]
enum Pending {
    Ship {
        ship: usize,
        task: usize,
        task_id: u64,
        ordinal: u32,
    },
    Failure(&'static str),
}

pub struct ShipNaming<'a> {
    pub ship_names: &'a mut Vec<u8>,
    pub name_len: &'a mut usize,
    pub name_required: &'a mut usize,
    pub ship_overrides: &'a mut Vec<u8>,
    pub override_len: &'a mut usize,
    pub override_required: &'a mut usize,
    pub quiet: fn() -> Option<(u64, u64)>,
}

struct ShipRun {
    pending: [Pending; SHIP_RUN],
    len: usize,
    pointers: [usize; SHIP_RUN],
    records: [u8; SHIP_RUN * cship::ROW_EXTENDED_STRIDE],
    members: crate::cname_group_member::MemberBatch,
    equipment: cship::EquipmentMemo,
}

impl ShipRun {
    fn new() -> Self {
        Self {
            pending: [Pending::Failure(""); SHIP_RUN],
            len: 0,
            pointers: [0; SHIP_RUN],
            records: [0; SHIP_RUN * cship::ROW_EXTENDED_STRIDE],
            members: crate::cname_group_member::MemberBatch::default(),
            equipment: cship::EquipmentMemo::default(),
        }
    }

    fn push(
        &mut self,
        entry: Pending,
        memory: &impl Memory,
        base: usize,
        ships: &mut Output<'_, cship::Row>,
        visit: &mut impl FnMut(usize, &mut cship::Row),
        naming: &mut Option<ShipNaming<'_>>,
    ) {
        if self.len == SHIP_RUN {
            self.flush(memory, base, ships, visit, naming);
        }
        self.pending[self.len] = entry;
        self.len += 1;
    }

    fn flush(
        &mut self,
        memory: &impl Memory,
        base: usize,
        ships: &mut Output<'_, cship::Row>,
        visit: &mut impl FnMut(usize, &mut cship::Row),
        naming: &mut Option<ShipNaming<'_>>,
    ) {
        let mut count = 0;
        for entry in &self.pending[..self.len] {
            if let Pending::Ship { ship, .. } = *entry {
                self.pointers[count] = ship;
                count += 1;
            }
        }
        let expected = engine_sites::at(base, cship::VTABLE_RVA).map(|vtable| vtable as u64);
        let mut crew_token = None;
        let mut crew_stable = false;
        let mut extended = false;
        let complete = if count != 0 && ships.may_take() {
            if let Some(naming) = naming.as_ref() {
                crew_token = (naming.quiet)();
                let ext = memory.copy_records(
                    engine_native_memory::memory::RecordSources::Pointers(&self.pointers[..count]),
                    &cship::ROW_EXTENDED_LAYOUT,
                    &mut self.records[..count * cship::ROW_EXTENDED_STRIDE],
                ) / cship::ROW_EXTENDED_FIELDS.len();
                crew_stable = crew_token.is_some() && (naming.quiet)() == crew_token;
                if ext == count {
                    extended = true;
                    ext
                } else {
                    memory.copy_records(
                        engine_native_memory::memory::RecordSources::Pointers(&self.pointers[..count]),
                        &cship::ROW_LAYOUT,
                        &mut self.records[..count * cship::ROW_STRIDE],
                    ) / cship::ROW_FIELDS.len()
                }
            } else {
                memory.copy_records(
                    engine_native_memory::memory::RecordSources::Pointers(&self.pointers[..count]),
                    &cship::ROW_LAYOUT,
                    &mut self.records[..count * cship::ROW_STRIDE],
                ) / cship::ROW_FIELDS.len()
            }
        } else {
            0
        };
        let mut member_ptrs = [0_usize; SHIP_RUN];
        if extended && let Some(expected_vtable) = expected {
            for at in 0..count {
                let rec = &self.records
                    [at * cship::ROW_EXTENDED_STRIDE..(at + 1) * cship::ROW_EXTENDED_STRIDE];
                let vtable = u64::from_le_bytes(rec[0..8].try_into().unwrap());
                let handle = u64::from_le_bytes(rec[8..16].try_into().unwrap());
                if vtable == expected_vtable
                    && handle != 0
                    && handle as u32 == cship::HANDLE_TYPE
                {
                    member_ptrs[at] =
                        usize::try_from(u64::from_le_bytes(rec[72..80].try_into().unwrap()))
                            .unwrap_or(0);
                }
            }
            self.members.capture(memory, &member_ptrs[..count]);
        }
        let stride = if extended {
            cship::ROW_EXTENDED_STRIDE
        } else {
            cship::ROW_STRIDE
        };
        let mut index = 0;
        let mut fell_back = false;
        for entry in &self.pending[..self.len] {
            match *entry {
                Pending::Failure(reason) => {
                    ships.error.get_or_insert(reason);
                }
                Pending::Ship {
                    ship,
                    task,
                    task_id,
                    ordinal,
                } => {
                    let at = index;
                    index += 1;
                    match ships.needs_row() {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(reason) => {
                            ships.error.get_or_insert(reason);
                            continue;
                        }
                    }
                    let mut row = match expected {
                        Some(expected) if at < complete => cship::decode_row(
                            &self.records[at * stride..at * stride + cship::ROW_STRIDE],
                            expected,
                            task,
                            task_id,
                        ),
                        _ => {
                            fell_back = true;
                            cship::read_row(memory, base, ship, task, task_id)
                        }
                    }
                    .map(|mut row| {
                        row.roster_ordinal = ordinal;
                        row
                    });
                    if let Ok(row) = &mut row {
                        if let Some(naming) = naming.as_mut() {
                            let captured = if extended
                                && at < complete
                                && (crew_stable || crew_token.is_none())
                            {
                                let rec = &self.records[at * cship::ROW_EXTENDED_STRIDE
                                    ..(at + 1) * cship::ROW_EXTENDED_STRIDE];
                                let crew = crew_stable.then(|| {
                                    [
                                        i32::from_le_bytes(rec[64..68].try_into().unwrap()),
                                        i32::from_le_bytes(rec[68..72].try_into().unwrap()),
                                    ]
                                });
                                let member_ptr = member_ptrs[at];
                                let header = if member_ptr != 0 {
                                    self.members.member(memory, at, member_ptr)
                                } else {
                                    Err("ship cached name member pointer unreadable or null")
                                };
                                cship::decode_batched_names_and_crew(
                                    memory,
                                    base,
                                    member_ptr,
                                    header,
                                    crew,
                                    naming.ship_names,
                                    naming.name_len,
                                    naming.name_required,
                                    naming.ship_overrides,
                                    naming.override_len,
                                    naming.override_required,
                                    &mut self.equipment,
                                )
                            } else {
                                cship::capture_names_and_crew(
                                    memory,
                                    base,
                                    ship,
                                    row.id,
                                    naming.ship_names,
                                    naming.name_len,
                                    naming.name_required,
                                    naming.ship_overrides,
                                    naming.override_len,
                                    naming.override_required,
                                    naming.quiet,
                                )
                            };
                            row.name = captured.name;
                            if let Some((_, len)) = row.name {
                                *naming.name_len += len;
                            }
                            row.naming = captured.naming;
                            if let Some(op) = row.naming {
                                *naming.override_len += op.override_name.1;
                            }
                            row.acquired_manpower = captured.crew.map(|value| value[0]);
                            row.required_manpower = captured.crew.map(|value| value[1]);
                        }
                        visit(ship, row);
                    }
                    ships.push(row);
                }
            }
        }
        if fell_back {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::ShipRuns);
        }
        self.len = 0;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet, task force, ship and name arenas are caller-owned, and the clock and visit callbacks belong to the caller"
)]
pub fn walk_timed_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &mut cship::Row),
    names: &mut Vec<u8>,
) -> TimedCapture {
    walk_named(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(fleets),
        engine_native_memory::walk::Buffer::Growing(task_forces),
        engine_native_memory::walk::Buffer::Growing(ships),
        now,
        visit,
        Some(names),
        true,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet, task force, ship and name buffers are caller-owned, the clock and visit callbacks belong to the caller, and batching is a flag"
)]
fn walk_named(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: engine_native_memory::walk::Buffer<'_, Row>,
    task_forces: engine_native_memory::walk::Buffer<'_, ctask_force::Row>,
    ships: engine_native_memory::walk::Buffer<'_, cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &mut cship::Row),
    names: Option<&mut Vec<u8>>,
    batched: bool,
) -> TimedCapture {
    walk_named_observed(
        memory,
        base,
        game_state,
        fleets,
        task_forces,
        ships,
        now,
        visit,
        names,
        batched,
        |_| {},
        None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet, task force, ship and name arenas are caller-owned, and the clock, visit and management callbacks belong to the caller"
)]
pub fn walk_timed_with_management(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &mut cship::Row),
    names: &mut Vec<u8>,
    management: impl FnMut(CensusVisit),
) -> TimedCapture {
    walk_named_observed(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(fleets),
        engine_native_memory::walk::Buffer::Growing(task_forces),
        engine_native_memory::walk::Buffer::Growing(ships),
        now,
        visit,
        Some(names),
        true,
        management,
        None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Batches fleet, task force, ship, naming, and naval management census in one walk."
)]
pub fn walk_timed_with_management_and_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &mut cship::Row),
    names: &mut Vec<u8>,
    management: impl FnMut(CensusVisit),
    ship_naming: ShipNaming<'_>,
) -> TimedCapture {
    walk_named_observed(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(fleets),
        engine_native_memory::walk::Buffer::Growing(task_forces),
        engine_native_memory::walk::Buffer::Growing(ships),
        now,
        visit,
        Some(names),
        true,
        management,
        Some(ship_naming),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet, task force, ship and name buffers are caller-owned, the clock, visit, management and naming hooks belong to the caller, and batching is a flag"
)]
fn walk_named_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: engine_native_memory::walk::Buffer<'_, Row>,
    task_forces: engine_native_memory::walk::Buffer<'_, ctask_force::Row>,
    ships: engine_native_memory::walk::Buffer<'_, cship::Row>,
    mut now: impl FnMut(usize) -> u64,
    mut visit: impl FnMut(usize, &mut cship::Row),
    mut names: Option<&mut Vec<u8>>,
    batched: bool,
    mut management: impl FnMut(CensusVisit),
    mut ship_naming: Option<ShipNaming<'_>>,
) -> TimedCapture {
    let mut names_used = 0;
    let invalid_reference = if names.is_some() {
        engine_sites::at(base, engine_native_parts::creference_registry::INVALID_REFERENCE_RVA)
            .and_then(|address| read::<u64>(memory, address))
    } else {
        None
    };
    let start = now(0);
    let mut read_ticks = [0_u64; 3];
    let source = memory;
    let fleet_view = cship::SwitchedObject::new(source, FLEET_FIELDS);
    let view = cship::SwitchedObject::new(&fleet_view, TASK_CENSUS_FIELDS);
    let memory = &view;
    let mut ship_output = Output::from_buffer(ships);
    let mut run = ShipRun::new();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let provinces = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|p| Array::read(memory, p, 8))
            .filter(|a| a.count >= 2)
            .ok_or("province array unreadable or invalid")?;
        let task_reader = ctask_force::Reader {
            memory,
            base,
            provinces,
        };
        let mut fleet_output = Output::from_buffer(fleets);
        let mut task_output = Output::from_buffer(task_forces);
        let mut found_country = false;
        let expected_fleet_vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            management(CensusVisit::Country(country));
            let fleet_roster = country
                .pointer
                .checked_add(ccountry::FLEETS)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("fleet roster unreadable or invalid")?;
            for index in 0..fleet_roster.count {
                let fleet = pointer(memory, fleet_roster, index)?;
                if batched {
                    fleet_view.select(fleet);
                    if !fleet_view.complete() {
                        engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::FleetGathers);
                    }
                }
                if read::<u64>(memory, fleet) != Some(expected_fleet_vtable as u64) {
                    return Err("fleet vtable mismatch");
                }
                let at = |offset| fleet.checked_add(offset).ok_or("fleet address overflow");
                let id = read::<u64>(memory, at(HANDLE)?).ok_or("fleet handle unreadable")?;
                if id == 0 {
                    return Err("non-null fleet has null handle");
                }
                let owner_pointer = read::<u64>(memory, at(OWNER)?)
                    .and_then(|p| usize::try_from(p).ok())
                    .ok_or("fleet owner unreadable")?;
                if owner_pointer != country.pointer {
                    return Err("fleet owner differs from country roster");
                }
                management(CensusVisit::Fleet {
                    pointer: fleet,
                    handle: id,
                });
                let tasks = Array::read(memory, at(TASK_FORCES)?, 8)
                    .ok_or("task force roster unreadable or invalid")?;
                if fleet_output.needs_row()? {
                    fleet_output.push((|| {
                        let home = read::<u64>(memory, at(HOME_BASE)?)
                            .and_then(|p| usize::try_from(p).ok())
                            .ok_or("fleet home base unreadable")?;
                        let mut row = Row {
                            id,
                            owner: country.id,
                            home_base: province(memory, base, provinces, home)?,
                            ..Default::default()
                        };
                        if let Some(arena) = names.as_deref_mut() {
                            row.leader =
                                read::<u64>(memory, at(LEADER)?).zip(invalid_reference).map(
                                    |(reference, invalid)| {
                                        if reference == invalid { 0 } else { reference }
                                    },
                                );
                            if let Ok((start, len)) = engine_native_memory::msvc_string::append(
                                memory,
                                at(NAME)?,
                                arena,
                                &mut names_used,
                            ) {
                                row.name_start = start;
                                row.name_len = len;
                                row.name_known = true;
                            }
                        }
                        Ok(row)
                    })());
                }
                for task_index in 0..tasks.count {
                    let task_start = now(1);
                    let task_result = (|| {
                        let task = pointer(memory, tasks, task_index)?;
                        if batched {
                            view.select(task);
                            if !view.complete() {
                                engine_native_memory::batch_fallback::record(
                                    engine_native_memory::batch_fallback::Batch::TaskForceGathers,
                                );
                            }
                        }
                        let task_id = task_reader.identity(task, fleet)?;
                        management(CensusVisit::TaskForce {
                            pointer: task,
                            handle: task_id,
                        });
                        if task_output.needs_row()? {
                            task_output.push(task_reader.row(task, task_id, id));
                        }
                        Ok::<_, &'static str>((task, task_id))
                    })();
                    read_ticks[1] = read_ticks[1].saturating_add(now(0).saturating_sub(task_start));
                    let (task, task_id) = match task_result {
                        Ok(task) => task,
                        Err(reason) => {
                            task_output.error.get_or_insert(reason);
                            if batched {
                                let ship_start = now(2);
                                run.push(
                                    Pending::Failure(reason),
                                    source,
                                    base,
                                    &mut ship_output,
                                    &mut visit,
                                    &mut ship_naming,
                                );
                                read_ticks[2] =
                                    read_ticks[2].saturating_add(now(0).saturating_sub(ship_start));
                            } else {
                                ship_output.error.get_or_insert(reason);
                            }
                            continue;
                        }
                    };
                    let ship_start = now(2);
                    let ship_result = (|| {
                        let ship_roster = task
                            .checked_add(ctask_force::SHIPS)
                            .and_then(|p| Array::read(memory, p, 8))
                            .ok_or("ship roster unreadable or invalid")?;
                        engine_native_memory::cpdx_array::pointer_pages(
                            memory,
                            ship_roster,
                            ROSTER_POINTER,
                            |first, page| {
                                if batched {
                                    for (index, &ship) in page.iter().enumerate() {
                                        if ship == 0 {
                                            return Err(ROSTER_POINTER);
                                        }
                                        cship::prefetch_row(ship);
                                        run.push(
                                            Pending::Ship {
                                                ship,
                                                task,
                                                task_id,
                                                ordinal: first + index as u32,
                                            },
                                            source,
                                            base,
                                            &mut ship_output,
                                            &mut visit,
                                            &mut ship_naming,
                                        );
                                    }
                                    return Ok(());
                                }
                                for &ship in page.iter().take(SHIP_HINTS) {
                                    cship::prefetch_row(ship);
                                }
                                for (index, &ship) in page.iter().enumerate() {
                                    if let Some(&ahead) = page.get(index + SHIP_HINTS) {
                                        cship::prefetch_row(ahead);
                                    }
                                    if ship == 0 {
                                        return Err(ROSTER_POINTER);
                                    }
                                    if ship_output.needs_row()? {
                                        let ship_index = first + index as u32;
                                        let mut row = cship::read_row(
                                            memory, base, ship, task, task_id,
                                        )
                                        .map(|mut row| {
                                            row.roster_ordinal = ship_index;
                                            row
                                        });
                                        if let Ok(row) = &mut row {
                                            visit(ship, row);
                                        }
                                        ship_output.push(row);
                                    }
                                }
                                Ok(())
                            },
                        )
                    })();
                    if let Err(reason) = ship_result {
                        if batched {
                            run.push(
                                Pending::Failure(reason),
                                source,
                                base,
                                &mut ship_output,
                                &mut visit,
                                &mut ship_naming,
                            );
                        } else {
                            ship_output.error.get_or_insert(reason);
                        }
                    }
                    read_ticks[2] = read_ticks[2].saturating_add(now(0).saturating_sub(ship_start));
                }
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok((fleet_output.finish(), task_output.finish()))
    })();
    let ship_start = now(2);
    run.flush(source, base, &mut ship_output, &mut visit, &mut ship_naming);
    read_ticks[2] = read_ticks[2].saturating_add(now(0).saturating_sub(ship_start));
    let capture = match result {
        Ok((fleets, task_forces)) => Capture {
            fleets,
            task_forces,
            ships: ship_output.finish(),
        },
        Err(reason) => Capture {
            fleets: Outcome::Unavailable { reason },
            task_forces: Outcome::Unavailable { reason },
            ships: Outcome::Unavailable { reason },
        },
    };
    read_ticks[0] = now(0)
        .saturating_sub(start)
        .saturating_sub(read_ticks[1])
        .saturating_sub(read_ticks[2]);
    if let Some(names) = names {
        names.truncate(names_used);
    }
    TimedCapture {
        capture,
        read_ticks,
    }
}

const ROSTER_POINTER: &str = "naval roster pointer unreadable or null";

fn pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or(ROSTER_POINTER)
}

pub fn province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    pointer: usize,
) -> Result<u32, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let vtable = engine_sites::at(base, cprovince::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(vtable as u64) {
        return Err("naval province vtable mismatch");
    }
    let id = pointer
        .checked_add(cprovince::ID)
        .and_then(|p| read::<u32>(memory, p))
        .filter(|&id| id != 0)
        .ok_or("naval province id unreadable or zero")?;
    if provinces
        .element(id, 8)
        .and_then(|p| read::<u64>(memory, p))
        != Some(pointer as u64)
    {
        return Err("naval province differs from registered slot");
    }
    Ok(id)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CounterRow {
    pub id: u64,
    pub rgba: [u32; 4],
    pub mission_region_count: i32,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterWalk {
    pub fleets: Outcome,
    pub task_forces: Outcome,
    pub roster_len: usize,
    pub roster_required: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskForceVisit {
    pub country: engine_native_memory::country_identity::Country,
    pub task: usize,
    pub id: u64,
    pub fleet: usize,
    pub fleet_handle: u64,
    pub leader: u64,
    pub kind: u32,
}

pub enum Visit {
    Fleet {
        country: engine_native_memory::country_identity::Country,
        fleet: usize,
        handle: u64,
        tasks: Array,
    },
    TaskForce(TaskForceVisit, u32),
}

pub fn visit_task_forces<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    visit_task_forces_inner(memory, base, countries, true, |_| (), visit)
}

pub fn visit_fuel_task_forces<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    visit_task_forces_inner(memory, base, countries, false, |_| (), visit)
}

fn visit_task_forces_inner<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    body_fields: bool,
    mut select: impl FnMut(usize),
    mut visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let fleet_vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let primary = engine_sites::at(base, ctask_force::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary = engine_sites::at(base, ctask_force::SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let fleet_fields = packed_fields([
        (0, 8),
        (HANDLE, 8),
        (OWNER, 8),
        (TASK_FORCES, 16),
        (LEADER, 8),
    ]);
    let fleet_layout = RecordLayout::new(
        &fleet_fields[..if body_fields { 5 } else { 4 }],
        if body_fields { 48 } else { 40 },
    )
    .expect("fleet identity layout");
    let task_fields = packed_fields([
        (0, 8),
        (ctask_force::SECONDARY_BASE, 8),
        (crate::cunit::HANDLE, 8),
        (ctask_force::FLEET, 8),
        (ctask_force::MISSION, 4),
    ]);
    let task_layout = RecordLayout::new(
        &task_fields[..if body_fields { 5 } else { 4 }],
        if body_fields { 36 } else { 32 },
    )
    .expect("task force identity layout");
    let fleet_stride = if body_fields { 48 } else { 40 };
    let fleet_field_count = if body_fields { 5 } else { 4 };
    let task_stride = if body_fields { 36 } else { 32 };
    let task_field_count = if body_fields { 5 } else { 4 };
    let mut fleet_buf = [0_u8; 64 * 48];
    let mut task_buf = [0_u8; 64 * 36];
    let mut found = false;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found = true;
        let fleets = country
            .pointer
            .checked_add(ccountry::FLEETS)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("fleet roster unreadable or invalid")?;
        engine_native_memory::cpdx_array::pointer_pages(
            memory,
            fleets,
            "fleet roster pointer unreadable or null",
            |_, fleet_page| {
                let fleets_valid = fleet_page
                    .iter()
                    .all(|&fleet| fleet != 0 && fleet.checked_add(TASK_FORCES).is_some());
                let complete_fleets = if fleets_valid && fleet_page.len() <= 64 {
                    let copied = memory.copy_records(
                        RecordSources::Pointers(fleet_page),
                        &fleet_layout,
                        &mut fleet_buf[..fleet_page.len() * fleet_stride],
                    );
                    copied / fleet_field_count
                } else {
                    0
                };
                for (fleet_offset, &fleet) in fleet_page.iter().enumerate() {
                    let (fleet_handle, tasks, leader) = if fleet_offset < complete_fleets {
                        let rec = &fleet_buf
                            [fleet_offset * fleet_stride..(fleet_offset + 1) * fleet_stride];
                        let value = |at| u64::from_le_bytes(rec[at..at + 8].try_into().unwrap());
                        if value(0) != fleet_vtable {
                            return Err("fleet vtable mismatch");
                        }
                        let fleet_handle = value(8);
                        if fleet_handle == 0 {
                            return Err("non-null fleet has null handle");
                        }
                        if value(16) != country.pointer as u64 {
                            return Err("fleet owner differs from country roster");
                        }
                        let tasks = Array::from_header(rec[24..40].try_into().unwrap(), 8)
                            .ok_or("task force roster unreadable or invalid")?;
                        let leader = if body_fields { value(40) } else { 0 };
                        (fleet_handle, tasks, leader)
                    } else {
                        if fleet == 0 {
                            return Err("fleet roster pointer unreadable or null");
                        }
                        fleet
                            .checked_add(TASK_FORCES)
                            .ok_or("fleet address overflow")?;
                        let mut bytes = [0; 48];
                        let copied = memory.copy_records(
                            RecordSources::Pointers(&[fleet]),
                            &fleet_layout,
                            &mut bytes,
                        );
                        let value = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
                        if copied < 1 || value(0) != fleet_vtable {
                            return Err("fleet vtable mismatch");
                        }
                        if copied < 2 {
                            return Err("fleet handle unreadable");
                        }
                        let fleet_handle = value(8);
                        if fleet_handle == 0 {
                            return Err("non-null fleet has null handle");
                        }
                        if copied < 3 || value(16) != country.pointer as u64 {
                            return Err("fleet owner differs from country roster");
                        }
                        if copied < 4 {
                            return Err("task force roster unreadable or invalid");
                        }
                        let tasks = Array::from_header(bytes[24..40].try_into().unwrap(), 8)
                            .ok_or("task force roster unreadable or invalid")?;
                        if body_fields && copied < 5 {
                            return Err("fleet leader unreadable");
                        }
                        (fleet_handle, tasks, value(40))
                    };
                    visit(Visit::Fleet {
                        country,
                        fleet,
                        handle: fleet_handle,
                        tasks,
                    })?;
                    engine_native_memory::cpdx_array::pointer_pages(
                        memory,
                        tasks,
                        "task force pointer unreadable or null",
                        |first, page| {
                            let check_offset = if body_fields {
                                ctask_force::MISSION
                            } else {
                                ctask_force::FLEET
                            };
                            let tasks_valid = page
                                .iter()
                                .all(|&task| task != 0 && task.checked_add(check_offset).is_some());
                            let complete_tasks = if tasks_valid && page.len() <= 64 {
                                let copied = memory.copy_records(
                                    RecordSources::Pointers(page),
                                    &task_layout,
                                    &mut task_buf[..page.len() * task_stride],
                                );
                                copied / task_field_count
                            } else {
                                0
                            };
                            for (offset, &task) in page.iter().enumerate() {
                                if let Some(&next) = page.get(offset + 1)
                                    && body_fields
                                {
                                    for (at, len) in ctask_force::TASK_FIELDS {
                                        engine_native_memory::prefetch::span(
                                            next.wrapping_add(at),
                                            len,
                                        );
                                    }
                                }
                                let (id, kind) = if offset < complete_tasks {
                                    let rec = &task_buf
                                        [offset * task_stride..(offset + 1) * task_stride];
                                    let value =
                                        |at| u64::from_le_bytes(rec[at..at + 8].try_into().unwrap());
                                    if value(0) != primary || value(8) != secondary {
                                        return Err("task force vtable mismatch");
                                    }
                                    let id = value(16);
                                    if id == 0 {
                                        return Err("non-null task force has null handle");
                                    }
                                    if value(24) != fleet as u64 {
                                        return Err("task force parent differs from fleet roster");
                                    }
                                    let kind = if body_fields {
                                        u32::from_le_bytes(rec[32..36].try_into().unwrap())
                                    } else {
                                        0
                                    };
                                    (id, kind)
                                } else {
                                    if task == 0 {
                                        return Err("task force pointer unreadable or null");
                                    }
                                    task.checked_add(check_offset)
                                        .ok_or("task force address overflow")?;
                                    let mut bytes = [0; 36];
                                    let copied = memory.copy_records(
                                        RecordSources::Pointers(&[task]),
                                        &task_layout,
                                        &mut bytes,
                                    );
                                    let value = |at| {
                                        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
                                    };
                                    if copied < 2 || value(0) != primary || value(8) != secondary {
                                        return Err("task force vtable mismatch");
                                    }
                                    if copied < 3 {
                                        return Err("task force handle unreadable");
                                    }
                                    let id = value(16);
                                    if id == 0 {
                                        return Err("non-null task force has null handle");
                                    }
                                    if copied < 4 || value(24) != fleet as u64 {
                                        return Err("task force parent differs from fleet roster");
                                    }
                                    if body_fields && copied < 5 {
                                        return Err("task force mission unreadable");
                                    }
                                    (id, u32::from_le_bytes(bytes[32..36].try_into().unwrap()))
                                };
                                select(task);
                                visit(Visit::TaskForce(
                                    TaskForceVisit {
                                        country,
                                        task,
                                        id,
                                        fleet,
                                        fleet_handle,
                                        leader,
                                        kind,
                                    },
                                    first + offset as u32,
                                ))?;
                            }
                            Ok(())
                        },
                    )?;
                }
                Ok(())
            },
        )?;
    }
    if !found {
        return Err("country table has no readable countries");
    }
    Ok(())
}

pub fn walk_counter_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    task_forces: &mut Vec<ctask_force::CounterRow>,
    visit: impl FnMut(usize, u64),
) -> CounterWalk {
    walk_counter_inputs_with_names(
        memory,
        base,
        game_state,
        fleets,
        rosters,
        (task_forces, None),
        visit,
    )
}

pub fn walk_counter_inputs_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    (task_forces, names): (
        &mut Vec<ctask_force::CounterRow>,
        Option<&mut crate::cname_group_member::Names<'_>>,
    ),
    visit: impl FnMut(usize, u64),
) -> CounterWalk {
    walk_counter_inputs_with_tooltips(
        memory,
        base,
        game_state,
        fleets,
        rosters,
        (task_forces, names, None),
        visit,
    )
}

pub fn walk_counter_inputs_with_tooltips(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    (task_forces, names, supply): (
        &mut Vec<ctask_force::CounterRow>,
        Option<&mut crate::cname_group_member::Names<'_>>,
        Option<&mut ctask_force::SupplyOrigins<'_>>,
    ),
    visit: impl FnMut(usize, u64),
) -> CounterWalk {
    walk_counter_inputs_with_port(
        memory,
        base,
        game_state,
        fleets,
        rosters,
        (task_forces, names, supply, None),
        visit,
    )
}

type PortCounterBuffers<'rows, 'names, 'bytes, 'supply, 'origins, 'port, 'sources, M> = (
    engine_native_memory::walk::Buffer<'rows, ctask_force::CounterRow>,
    Option<&'names mut crate::cname_group_member::Names<'bytes>>,
    Option<&'supply mut ctask_force::SupplyOrigins<'origins>>,
    Option<&'port mut ctask_force::port_mission::Collector<'sources, M>>,
);

type PortCounterInputs<'rows, 'names, 'bytes, 'supply, 'origins, 'port, 'sources, M> = (
    &'rows mut Vec<ctask_force::CounterRow>,
    Option<&'names mut crate::cname_group_member::Names<'bytes>>,
    Option<&'supply mut ctask_force::SupplyOrigins<'origins>>,
    Option<&'port mut ctask_force::port_mission::Collector<'sources, M>>,
);

pub fn walk_counter_inputs_with_port<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    inputs: PortCounterInputs<'_, '_, '_, '_, '_, '_, '_, M>,
    visit: impl FnMut(usize, u64),
) -> CounterWalk {
    walk_counter_inputs_with_port_observed(
        memory,
        base,
        game_state,
        fleets,
        rosters,
        inputs,
        visit,
        |_, _, _| {},
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleets and rosters are caller-owned, the port inputs are grouped, and the visit and observe callbacks belong to the caller"
)]
pub fn walk_counter_inputs_with_port_observed<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    inputs: PortCounterInputs<'_, '_, '_, '_, '_, '_, '_, M>,
    visit: impl FnMut(usize, u64),
    observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
) -> CounterWalk {
    walk_counter_inputs_with_port_and_fleet_observed(
        memory,
        base,
        game_state,
        fleets,
        rosters,
        inputs,
        (|_: usize| (), visit),
        observe,
        |_, _| (),
        |_, _, _, _| {},
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleets and rosters are caller-owned, the port inputs are grouped, and each callback belongs to the caller"
)]
pub fn walk_counter_inputs_with_port_and_fleet_observed<M: Memory, T>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    (task_forces, names, supply, port): PortCounterInputs<'_, '_, '_, '_, '_, '_, '_, M>,
    (select, visit): (impl FnMut(usize), impl FnMut(usize, u64)),
    observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
    fleet_regions_before: impl FnMut(usize, u64) -> T,
    fleet_regions: impl FnMut(usize, u64, Array, T),
) -> CounterWalk {
    walk_counter_buffers_with_port_observed(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(fleets),
        engine_native_memory::walk::Buffer::Growing(rosters),
        (
            engine_native_memory::walk::Buffer::Growing(task_forces),
            names,
            supply,
            port,
        ),
        (select, visit),
        observe,
        fleet_regions_before,
        fleet_regions,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fleet and roster buffers are caller-owned, the port buffers are grouped, and each callback belongs to the caller"
)]
fn walk_counter_buffers_with_port_observed<M: Memory, T>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: engine_native_memory::walk::Buffer<'_, CounterRow>,
    mut rosters: engine_native_memory::walk::Buffer<'_, u64>,
    (task_forces, mut names, mut supply, mut port): PortCounterBuffers<
        '_,
        '_,
        '_,
        '_,
        '_,
        '_,
        '_,
        M,
    >,
    (mut select, mut visit): (impl FnMut(usize), impl FnMut(usize, u64)),
    mut observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
    mut fleet_regions_before: impl FnMut(usize, u64) -> T,
    mut fleet_regions: impl FnMut(usize, u64, Array, T),
) -> CounterWalk {
    let mut roster_len = 0usize;
    let mut roster_required = 0_usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut fleet_output = Output::from_buffer(fleets);
        let mut task_output = Output::from_buffer(task_forces);
        let mut keep = false;
        let mut task_count = 0;
        visit_task_forces_inner(memory, base, &countries, true, &mut select, |event| {
            let finished = match event {
                Visit::Fleet {
                    fleet,
                    handle,
                    tasks,
                    ..
                } => {
                    roster_required = roster_required
                        .checked_add(tasks.count as usize)
                        .ok_or("fleet roster demand overflow")?;
                    keep = fleet_output.needs_row()?
                        && rosters.ensure(
                            roster_len
                                .checked_add(tasks.count as usize)
                                .ok_or("fleet roster sizing overflow")?,
                        )?;
                    if !keep && fleet_output.len < fleet_output.rows.len() {
                        fleet_output.unread = fleet_output
                            .unread
                            .checked_add(1)
                            .ok_or("naval unread row count overflow")?;
                    }
                    task_count = tasks.count;
                    (task_count == 0).then_some((fleet, handle))
                }
                Visit::TaskForce(task, index) => {
                    visit(task.task, task.id);
                    if keep {
                        rosters[roster_len + index as usize] = task.id;
                    }
                    if task_output.needs_row()? {
                        let row = ctask_force::read_counter_inputs(
                            memory, base, game_state, task.task, task.id,
                        )
                        .map(|mut row| {
                            if let Some(names) = names.as_deref_mut() {
                                row.name = ctask_force::cached_name(
                                    memory, base, task.task, task.id, names,
                                )
                                .ok()
                                .flatten();
                                if let Some((start, len)) = row.name {
                                    names.len = start + len;
                                }
                            }
                            if let Some(supply) = supply.as_deref_mut() {
                                row.supply_origins_start = supply.len;
                                match engine_native_parts::cunit_supply_inputs::capture_buffer(
                                    memory,
                                    task.task,
                                    supply.origins.suffix(supply.len),
                                ) {
                                    Ok(inputs) => {
                                        let demand = match inputs.origins_len {
                                            Ok(len) => {
                                                supply.len += len;
                                                len
                                            }
                                            Err(engine_native_parts::cunit_supply_inputs::Failure::OriginsCapacity { required }) => required,
                                            Err(_) => 0,
                                        };
                                        supply.required = supply.required.saturating_add(demand);
                                        row.supply_failure = inputs.origins_len.err();
                                        row.supply = Some(inputs);
                                    }
                                    Err(reason) => row.supply_failure = Some(reason),
                                }
                            }
                            if let Some(port) = port.as_deref_mut() {
                                row.port_mission = Some(port.capture(task.task, task.kind, &row));
                            }
                            row
                        });
                        observe(task.task, task.id, Some(row));
                        task_output.push(row);
                    } else {
                        observe(task.task, task.id, None);
                    }
                    (index + 1 == task_count).then_some((task.fleet, task.fleet_handle))
                }
            };
            if let Some((fleet, id)) = finished.filter(|_| keep) {
                let at = |offset| fleet.checked_add(offset).ok_or("fleet address overflow");
                let colour = read::<[u8; 16]>(memory, at(COLOR_COMPONENTS)?)
                    .ok_or("fleet colour unreadable")?;
                let region_token = fleet_regions_before(fleet, id);
                let regions = Array::read(memory, at(MISSION_REGIONS)?, 8)
                    .ok_or("fleet mission regions invalid")?;
                fleet_regions(fleet, id, regions, region_token);
                fleet_output.push(Ok(CounterRow {
                    id,
                    rgba: std::array::from_fn(|n| {
                        u32::from_le_bytes(colour[n * 4..n * 4 + 4].try_into().unwrap())
                    }),
                    mission_region_count: regions.count as i32,
                    start: roster_len,
                    len: task_count as usize,
                }));
                roster_len += task_count as usize;
            }
            Ok(())
        })?;
        Ok((fleet_output.finish(), task_output.finish()))
    })();
    match result {
        Ok((fleets, task_forces)) => CounterWalk {
            fleets,
            task_forces,
            roster_len,
            roster_required,
        },
        Err(reason) => CounterWalk {
            fleets: Outcome::Unavailable { reason },
            task_forces: Outcome::Unavailable { reason },
            roster_len: 0,
            roster_required,
        },
    }
}

#[cfg(test)]
mod tests;

pub fn read_mission_regions(
    memory: &impl Memory,
    base: usize,
    regions: Array,
) -> Result<Vec<u32>, &'static str> {
    let mut output = engine_allocation::vec(regions.count as usize)
        .map_err(|_| "fleet region allocation failed")?;
    for index in 0..regions.count {
        let pointer = regions
            .element(index, 8)
            .and_then(|at| read::<u64>(memory, at))
            .ok_or("fleet mission region pointer unreadable")?;
        let id = engine_native_parts::cstrategic_region::id(memory, base, pointer as usize)?
            .ok_or("fleet mission region is null")?;
        output.push(id);
    }
    output.sort_unstable();
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
pub enum CensusVisit {
    Country(engine_native_memory::country_identity::Country),
    Fleet { pointer: usize, handle: u64 },
    TaskForce { pointer: usize, handle: u64 },
}
