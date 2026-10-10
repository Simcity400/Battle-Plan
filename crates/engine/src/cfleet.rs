//! CFleet layout; task-force vector entries are complete CTaskForce pointers.

use crate::{
    Memory, ccountry, cgame_state,
    cpdx_array::Array,
    cprovince, cship, ctask_force,
    memory::{RecordLayout, RecordSources, packed_fields},
    read,
    walk::Outcome,
};

/// CFleet primary vtable.
pub const VTABLE_RVA: usize = 0x0294_bf28;
/// CFleet reference handle {u32 type, u32 id}.
pub const HANDLE: usize = 0x08;
/// CFleet reference type.
pub const HANDLE_TYPE: u32 = 61;
/// CFleet owner CCountry pointer.
pub const OWNER: usize = 0xa8;
/// Fleet leader reference.
pub const LEADER: usize = 0xb0;
/// Stored fleet MSVC name, filled during creation and saved directly.
pub const NAME: usize = 0xe0;
/// CFleet task-force pointer array.
pub const TASK_FORCES: usize = 0xb8;
/// CFleet task-force array capacity, u32.
pub const TASK_FORCE_CAPACITY: usize = 0xc0;
/// CFleet task-force array count, u32.
pub const TASK_FORCE_COUNT: usize = 0xc4;
/// CFleet home-base CProvince pointer.
pub const HOME_BASE: usize = 0xd8;
/// CFleet mission CStrategicRegion pointer array.
pub const MISSION_REGIONS: usize = 0x48;
/// CFleet mission region count i32.
pub const MISSION_REGION_COUNT: usize = 0x54;
/// Fleet embedded colour object.
pub const COLOR: usize = 0x110;
/// RGBA f32 components at colour+0x10, copied into the port counter.
pub const COLOR_COMPONENTS: usize = 0x120;
/// Fleet serializer writes leader, name, colour, icon and ordered task-force entries.
pub const SERIALIZE_RVA: usize = 0x00d4_a390;
/// Fleet reserve classifier tests its first task force's state against 8.
pub const IS_RESERVE_RVA: usize = 0x00d4_3440;
/// CFleet strip-eligibility helper.
pub const STRIP_ELIGIBILITY_RVA: usize = 0x00d4_5e00;

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
    /// Caller clock ticks for fleets/root traversal, task forces and ships.
    /// These add to the elapsed walk; clock/loop overhead stays in the total.
    pub read_ticks: [u64; 3],
}

struct Output<'a, R> {
    rows: crate::walk::Buffer<'a, R>,
    len: usize,
    unread: u32,
    error: Option<&'static str>,
}

impl<'a, R: Default> Output<'a, R> {
    fn from_buffer(rows: crate::walk::Buffer<'a, R>) -> Self {
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

/// Descends each canonical country's fleet/task-force/ship graph once. Empty
/// rosters are valid. Only root and relationship failures invalidate the whole
/// graph; a table's unreadable body fields remain that table's failure. Full
/// buffers stop body reads while the traversal counts omitted rows and reads
/// the roster metadata needed by the other tables. No sorting or allocation
/// occurs here; duplicate native handles are rejected during frame encoding.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [Row],
    task_forces: &mut [ctask_force::Row],
    ships: &mut [cship::Row],
) -> Capture {
    walk_timed(memory, base, game_state, fleets, task_forces, ships, |_| 0).capture
}

/// Measures the three tables without repeating the shared graph traversal.
/// The caller supplies a monotonic clock; each ship vector is timed as a batch.
/// Clock boundaries name the following phase: fleets/shared roots = 0,
/// task forces = 1, ships = 2. Every completed phase returns to 0, including
/// failures; the final boundary also names 0.
pub fn walk_timed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [Row],
    task_forces: &mut [ctask_force::Row],
    ships: &mut [cship::Row],
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

/// Shares authenticated emitted ship rows with independent state extensions.
/// A visitor receives the transient ship pointer, never a replay identity. Its
/// errors stay in its own table. A truncated or unavailable Ships outcome must
/// invalidate any dependent census, even if a visited prefix was readable.
/// Visitor cost is included in ship ticks; the caller attributes it separately.
#[allow(clippy::too_many_arguments)] // Three caller-owned tables plus a clock and visitor.
pub fn walk_timed_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [Row],
    task_forces: &mut [ctask_force::Row],
    ships: &mut [cship::Row],
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &cship::Row),
) -> TimedCapture {
    walk_named(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(fleets),
        crate::walk::Buffer::Fixed(task_forces),
        crate::walk::Buffer::Fixed(ships),
        now,
        visit,
        None,
    )
}

/// Worker-only acquisition of every fleet's owned name and raw leader reference.
/// The caller holds the game thread while this shared graph walk reads it.
/// Growth copies only bytes observed successfully, never a chosen name limit.
#[allow(clippy::too_many_arguments)]
pub fn walk_timed_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [Row],
    task_forces: &mut [ctask_force::Row],
    ships: &mut [cship::Row],
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &cship::Row),
    names: &mut Vec<u8>,
) -> TimedCapture {
    walk_named(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(fleets),
        crate::walk::Buffer::Fixed(task_forces),
        crate::walk::Buffer::Fixed(ships),
        now,
        visit,
        Some(names),
    )
}

/// Worker storage follows each original fleet/task/ship observation.
/// Task-force traversal has no chosen population ceiling and growth does not
/// repeat acquisition.
#[allow(clippy::too_many_arguments)]
pub fn walk_timed_growing_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<Row>,
    task_forces: &mut Vec<ctask_force::Row>,
    ships: &mut Vec<cship::Row>,
    now: impl FnMut(usize) -> u64,
    visit: impl FnMut(usize, &cship::Row),
    names: &mut Vec<u8>,
) -> TimedCapture {
    walk_named(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(fleets),
        crate::walk::Buffer::Growing(task_forces),
        crate::walk::Buffer::Growing(ships),
        now,
        visit,
        Some(names),
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_named(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: crate::walk::Buffer<'_, Row>,
    task_forces: crate::walk::Buffer<'_, ctask_force::Row>,
    ships: crate::walk::Buffer<'_, cship::Row>,
    mut now: impl FnMut(usize) -> u64,
    mut visit: impl FnMut(usize, &cship::Row),
    mut names: Option<&mut Vec<u8>>,
) -> TimedCapture {
    let mut names_used = 0;
    let invalid_reference = if names.is_some() {
        base.checked_add(crate::creference_registry::INVALID_REFERENCE_RVA)
            .and_then(|address| read::<u64>(memory, address))
    } else {
        None
    };
    let start = now(0);
    let mut read_ticks = [0_u64; 3];
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
            countries: &countries,
            provinces,
        };
        let mut fleet_output = Output::from_buffer(fleets);
        let mut task_output = Output::from_buffer(task_forces);
        let mut ship_output = Output::from_buffer(ships);
        let mut found_country = false;
        let expected_fleet_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let fleet_roster = country
                .pointer
                .checked_add(ccountry::FLEETS)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("fleet roster unreadable or invalid")?;
            for index in 0..fleet_roster.count {
                let fleet = pointer(memory, fleet_roster, index)?;
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
                            // Compare both handle words with the live invalid-reference object
                            // before retaining the leader.
                            row.leader =
                                read::<u64>(memory, at(LEADER)?).zip(invalid_reference).map(
                                    |(reference, invalid)| {
                                        if reference == invalid { 0 } else { reference }
                                    },
                                );
                            if let Ok((start, len)) = crate::msvc_string::append_growing(
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
                        let task_id = task_reader.identity(task, fleet)?;
                        if task_output.needs_row()? {
                            task_output.push(task_reader.row(
                                task,
                                task_id,
                                id,
                                country.canonical_slot,
                            ));
                        }
                        Ok::<_, &'static str>((task, task_id))
                    })();
                    read_ticks[1] = read_ticks[1].saturating_add(now(0).saturating_sub(task_start));
                    let (task, task_id) = match task_result {
                        Ok(task) => task,
                        Err(reason) => {
                            task_output.error.get_or_insert(reason);
                            ship_output.error.get_or_insert(reason);
                            continue;
                        }
                    };
                    let ship_start = now(2);
                    let ship_result = (|| {
                        let ship_roster = task
                            .checked_add(ctask_force::SHIPS)
                            .and_then(|p| Array::read(memory, p, 8))
                            .ok_or("ship roster unreadable or invalid")?;
                        for ship_index in 0..ship_roster.count {
                            let ship = pointer(memory, ship_roster, ship_index)?;
                            if ship_output.needs_row()? {
                                let row = cship::read_row(memory, base, ship, task, task_id).map(
                                    |mut row| {
                                        row.roster_ordinal = ship_index;
                                        row
                                    },
                                );
                                if let Ok(row) = &row {
                                    visit(ship, row);
                                }
                                ship_output.push(row);
                            }
                        }
                        Ok::<_, &'static str>(())
                    })();
                    read_ticks[2] = read_ticks[2].saturating_add(now(0).saturating_sub(ship_start));
                    if let Err(reason) = ship_result {
                        ship_output.error.get_or_insert(reason);
                    }
                }
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(Capture {
            fleets: fleet_output.finish(),
            task_forces: task_output.finish(),
            ships: ship_output.finish(),
        })
    })();
    let capture = result.unwrap_or_else(|reason| Capture {
        fleets: Outcome::Unavailable { reason },
        task_forces: Outcome::Unavailable { reason },
        ships: Outcome::Unavailable { reason },
    });
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

fn pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("naval roster pointer unreadable or null")
}

pub(crate) fn province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    pointer: usize,
) -> Result<u32, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let vtable = base
        .checked_add(cprovince::VTABLE_RVA)
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

/// Fleet marker colour bits, mission-region count and ordered task-force
/// roster in the caller's handle arena.
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
    /// Roster handles written, and handles the complete census needs.
    pub roster_len: usize,
    pub roster_required: usize,
}

/// A task force found by [`visit_task_forces`], already authenticated, with its fleet (whose pointer
/// `CTaskForce+0x340` the walk checked equal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskForceVisit {
    pub country: ccountry::Country,
    pub task: usize,
    pub id: u64,
    pub fleet: usize,
    pub fleet_handle: u64,
    pub leader: u64,
    pub kind: u32,
}

/// An authenticated fleet followed by its task forces in roster order.
pub enum Visit {
    Fleet {
        country: ccountry::Country,
        fleet: usize,
        handle: u64,
        tasks: Array,
    },
    TaskForce(TaskForceVisit, u32),
}

/// Visits every task force of every canonical country's fleets (`CCountry+0x278`,
/// `CFleet+0xB8`), in country, fleet and roster order. Each fleet's vtable, handle and owner
/// and each task force's vtables, handle and parent fleet are checked, as
/// `walk_counter_inputs` checks them, before its kind is trusted. The callback receives each
/// fleet (including empty fleets), then its task forces with their roster indices.
pub fn visit_task_forces<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    visit_task_forces_inner(memory, base, countries, true, visit)
}

/// Use the same authenticated roster for cached fuel. Leader and mission
/// body faults do not withhold a cache whose getter never reads them.
pub fn visit_fuel_task_forces<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    visit_task_forces_inner(memory, base, countries, false, visit)
}

fn visit_task_forces_inner<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    body_fields: bool,
    mut visit: impl FnMut(Visit) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let fleet_vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let primary = base
        .checked_add(ctask_force::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary = base
        .checked_add(ctask_force::SECONDARY_VTABLE_RVA)
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
        crate::cpdx_array::pointers(
            memory,
            fleets,
            "fleet roster pointer unreadable or null",
            |_, fleet| {
                if fleet == 0 {
                    return Err("fleet roster pointer unreadable or null");
                }
                // Preserve the named overflow error before copy_records reports a short prefix.
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
                // Interpret the gathered prefix in the established authentication order.
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
                let leader = value(40);
                visit(Visit::Fleet {
                    country,
                    fleet,
                    handle: fleet_handle,
                    tasks,
                })?;
                crate::cpdx_array::pointers(
                    memory,
                    tasks,
                    "task force pointer unreadable or null",
                    |index, task| {
                        if task == 0 {
                            return Err("task force pointer unreadable or null");
                        }
                        // Preserve the named overflow error before copy_records reports a short prefix.
                        task.checked_add(if body_fields {
                            ctask_force::MISSION
                        } else {
                            ctask_force::FLEET
                        })
                        .ok_or("task force address overflow")?;
                        let mut bytes = [0; 36];
                        let copied = memory.copy_records(
                            RecordSources::Pointers(&[task]),
                            &task_layout,
                            &mut bytes,
                        );
                        let value = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
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
                        visit(Visit::TaskForce(
                            TaskForceVisit {
                                country,
                                task,
                                id,
                                fleet,
                                fleet_handle,
                                leader,
                                kind: u32::from_le_bytes(bytes[32..36].try_into().unwrap()),
                            },
                            index,
                        ))
                    },
                )
            },
        )?;
    }
    if !found {
        return Err("country table has no readable countries");
    }
    Ok(())
}

/// Descends each canonical country's fleets and task forces once, with the same authentication as
/// [`walk_timed_with`], and reads only the counter operands. A fleet whose roster does not fit is
/// omitted whole; task-force rows are independent of it. `visit` receives every authenticated
/// task force's pointer and handle, for censuses that share this walk.
pub fn walk_counter_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [CounterRow],
    rosters: &mut [u64],
    task_forces: &mut [ctask_force::CounterRow],
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

/// The same walk with an optional cached-name arena paired with task-force
/// rows. Acquire names after the counter prefix succeeds; name faults or
/// short arenas retain the row and all prefix fields. Preserve visitor
/// order and the no-name read trace.
pub fn walk_counter_inputs_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [CounterRow],
    rosters: &mut [u64],
    (task_forces, names): (
        &mut [ctask_force::CounterRow],
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

/// The established walk adds independent name and supply suffixes after
/// the primary task prefix succeeds. Without arenas, reads and visitor
/// ordering are unchanged. Suffix refusal cannot change counter, motion
/// or mission censuses.
pub fn walk_counter_inputs_with_tooltips(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    fleets: &mut [CounterRow],
    rosters: &mut [u64],
    (task_forces, names, supply): (
        &mut [ctask_force::CounterRow],
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

// Keep each row/loan/arena lifetime independent, as in the original tuple.
type PortCounterInputs<'rows, 'names, 'bytes, 'supply, 'origins, 'port, 'sources, M> = (
    &'rows mut [ctask_force::CounterRow],
    Option<&'names mut crate::cname_group_member::Names<'bytes>>,
    Option<&'supply mut ctask_force::SupplyOrigins<'origins>>,
    Option<&'port mut ctask_force::port_mission::Collector<'sources, M>>,
);

type PortCounterBuffers<'rows, 'names, 'bytes, 'supply, 'origins, 'port, 'sources, M> = (
    crate::walk::Buffer<'rows, ctask_force::CounterRow>,
    Option<&'names mut crate::cname_group_member::NameArena<'bytes>>,
    Option<&'supply mut ctask_force::SupplyOrigins<'origins>>,
    Option<&'port mut ctask_force::port_mission::Collector<'sources, M>>,
);

type GrowingPortCounterInputs<'rows, 'names, 'bytes, 'supply, 'origins, 'port, 'sources, M> = (
    &'rows mut Vec<ctask_force::CounterRow>,
    Option<&'names mut crate::cname_group_member::NameArena<'bytes>>,
    Option<&'supply mut ctask_force::SupplyOrigins<'origins>>,
    Option<&'port mut ctask_force::port_mission::Collector<'sources, M>>,
);

/// Optional political port operands after the admitted prefix/name/supply.
/// Every older entry point supplies None, preserving its reads and ordering.
/// The port collector owns independent prepared arenas and pass-local answers;
/// a refused port group cannot change any prior field or the task-force census.
pub fn walk_counter_inputs_with_port<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut [CounterRow],
    rosters: &mut [u64],
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

/// Optional borrower of the SAME original counter prefix, including refusal
/// and an unreached capacity result. It cannot retry the native counter read.
#[allow(clippy::too_many_arguments)]
pub fn walk_counter_inputs_with_port_observed<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut [CounterRow],
    rosters: &mut [u64],
    (task_forces, mut names, supply, port): PortCounterInputs<'_, '_, '_, '_, '_, '_, '_, M>,
    visit: impl FnMut(usize, u64),
    observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
) -> CounterWalk {
    let mut arena = names
        .as_deref_mut()
        .map(|names| crate::cname_group_member::NameArena {
            bytes: crate::walk::Buffer::Fixed(names.bytes),
            len: names.len,
            required: names.required,
        });
    let result = walk_counter_buffers_with_port_observed(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(fleets),
        crate::walk::Buffer::Fixed(rosters),
        (
            crate::walk::Buffer::Fixed(task_forces),
            arena.as_mut(),
            supply,
            port,
        ),
        visit,
        observe,
    );
    let counts = arena.map(|arena| (arena.len, arena.required));
    if let (Some(names), Some((len, required))) = (names, counts) {
        names.len = len;
        names.required = required;
    }
    result
}

/// Worker-only counters grow rows and ordered rosters from the original
/// array header without a population cap.
#[allow(clippy::too_many_arguments)]
pub fn walk_counter_inputs_with_port_observed_growing<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: &mut Vec<CounterRow>,
    rosters: &mut Vec<u64>,
    (task_forces, names, supply, port): GrowingPortCounterInputs<'_, '_, '_, '_, '_, '_, '_, M>,
    visit: impl FnMut(usize, u64),
    observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
) -> CounterWalk {
    walk_counter_buffers_with_port_observed(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(fleets),
        crate::walk::Buffer::Growing(rosters),
        (
            crate::walk::Buffer::Growing(task_forces),
            names,
            supply,
            port,
        ),
        visit,
        observe,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_counter_buffers_with_port_observed<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    fleets: crate::walk::Buffer<'_, CounterRow>,
    mut rosters: crate::walk::Buffer<'_, u64>,
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
    mut visit: impl FnMut(usize, u64),
    mut observe: impl FnMut(usize, u64, Option<Result<ctask_force::CounterRow, &'static str>>),
) -> CounterWalk {
    let mut roster_len = 0usize;
    let mut roster_required = 0_usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut fleet_output = Output::from_buffer(fleets);
        let mut task_output = Output::from_buffer(task_forces);
        let mut keep = false;
        let mut task_count = 0;
        visit_task_forces(memory, base, &countries, |event| {
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
                                row.name = ctask_force::cached_name_buffer(
                                    memory, base, task.task, task.id, names,
                                )
                                .ok()
                                .flatten();
                                if let Some((start, len)) = row.name {
                                    // copy_string proved this span is inside the arena.
                                    names.len = start + len;
                                }
                            }
                            if let Some(supply) = supply.as_deref_mut() {
                                row.supply_origins_start = supply.len;
                                match crate::cunit_supply_inputs::capture_buffer(
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
                                            Err(crate::cunit_supply_inputs::Failure::OriginsCapacity { required }) => required,
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
            // Preserve main's task authentication before fleet counter operand failures.
            if let Some((fleet, id)) = finished.filter(|_| keep) {
                let at = |offset| fleet.checked_add(offset).ok_or("fleet address overflow");
                let colour = read::<[u8; 16]>(memory, at(COLOR_COMPONENTS)?)
                    .ok_or("fleet colour unreadable")?;
                let regions = Array::read(memory, at(MISSION_REGIONS)?, 8)
                    .ok_or("fleet mission regions invalid")?;
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
