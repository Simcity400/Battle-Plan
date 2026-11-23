
use engine_sites::Rva;
use engine_native_memory::{Memory, cgame_date, cpdx_array::Array, read};
use engine_allocation as allocation;

pub const STATUS: usize = 0x620;
pub const VTABLE_RVA: Rva = Rva(0x029b_8d78);
pub const OWNER: usize = 0x08;
pub const MANIFEST: usize = 0x10;
pub const GOALS: usize = 0x18;
pub const COMPLETED: usize = 0x48;
pub const CANCELLED: usize = 0x60;
pub const EXTRA_SLOTS: usize = 0x78;
pub const TERM_DATES: usize = 0x140;
pub const GOAL_STRIDE: usize = 0x540;
pub const GOAL_DEFINITION: usize = 0;
pub const GOAL_STATUS: usize = 0x08;
pub const GOAL_PROGRESS_STATUS: usize = 0x10;
pub const GOAL_PROGRESS: usize = 0x520;
pub const GOAL_DATE: usize = 0x528;
pub const DEFINITION_VTABLE_RVA: Rva = Rva(0x0279_7990);
pub const DEFINITION_TOKEN: usize = 0x08;
pub const DATE_STRIDE: usize = 0x18;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Goal {
    pub token: u32,
    pub status: i32,
    pub progress_raw: i64,
    pub date: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub manifest: Option<u32>,
    pub goals: Vec<Goal>,
    pub goal_addresses: Vec<usize>,
    pub active_sections: Vec<Vec<u32>>,
    pub completed: Vec<Option<u32>>,
    pub cancelled: Vec<Option<u32>>,
    pub extra_slots: Vec<i32>,
    pub term_dates: Vec<i32>,
}

pub(crate) fn at(address: usize, offset: usize) -> Result<usize, &'static str> {
    address
        .checked_add(offset)
        .ok_or("faction goal address overflow")
}

fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    address: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(address, offset)?).ok_or("faction goal field unreadable")
}

fn array(
    memory: &impl Memory,
    address: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(address, offset)?, stride)
        .ok_or("faction goal array unreadable or invalid")
}

fn resize<T: Default>(output: &mut Vec<T>, count: u32) -> Result<(), &'static str> {
    allocation::resize_default(output, count as usize)
        .map_err(|_| "faction goal storage allocation failed")
}

fn definition(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<u32>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let vtable = engine_sites::at(base, DEFINITION_VTABLE_RVA)
        .ok_or("faction goal definition vtable mismatch")?;
    if read::<u64>(memory, pointer) != Some(vtable as u64) {
        return Err("faction goal definition vtable mismatch");
    }
    word(memory, pointer, DEFINITION_TOKEN).map(Some)
}

fn definitions(
    memory: &impl Memory,
    base: usize,
    source: Array,
    output: &mut Vec<Option<u32>>,
) -> Result<(), &'static str> {
    resize(output, source.count)?;
    for (ordinal, value) in output.iter_mut().enumerate() {
        let pointer = read::<u64>(
            memory,
            source
                .element(ordinal as u32, 8)
                .ok_or("faction goal list address overflow")?,
        )
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("faction goal definition pointer unreadable")?;
        *value = definition(memory, base, pointer)?;
    }
    Ok(())
}

fn date(memory: &impl Memory, base: usize, address: usize) -> Result<i32, &'static str> {
    cgame_date::read_raw(memory, base, address)
}

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    read_selected_into(memory, base, faction, output, true)
}

pub fn read_structure_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    read_selected_into(memory, base, faction, output, false)
}

fn read_selected_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Snapshot,
    progress: bool,
) -> Result<(), &'static str> {
    let status = at(faction, STATUS)?;
    let vtable =
        engine_sites::at(base, VTABLE_RVA).ok_or("faction goal status vtable mismatch")?;
    if read::<u64>(memory, status) != Some(vtable as u64) {
        return Err("faction goal status vtable mismatch");
    }
    if word::<u64>(memory, status, OWNER)? != faction as u64 {
        return Err("faction goal status owner mismatch");
    }
    let pointer = usize::try_from(word::<u64>(memory, status, MANIFEST)?)
        .map_err(|_| "faction manifest pointer invalid")?;
    output.manifest = definition(memory, base, pointer)?;
    let goals = array(memory, status, GOALS, GOAL_STRIDE)?;
    resize(&mut output.goals, goals.count)?;
    resize(&mut output.goal_addresses, goals.count)?;
    resize(&mut output.active_sections, goals.count)?;
    for (ordinal, goal) in output.goals.iter_mut().enumerate() {
        let address = goals
            .element(ordinal as u32, GOAL_STRIDE)
            .ok_or("faction goal address overflow")?;
        let pointer = usize::try_from(word::<u64>(memory, address, GOAL_DEFINITION)?)
            .map_err(|_| "faction goal pointer invalid")?;
        *goal = Goal {
            token: definition(memory, base, pointer)?
                .ok_or("faction runtime goal has no definition")?,
            status: word(memory, address, GOAL_STATUS)?,
            progress_raw: if progress {
                word(memory, address, GOAL_PROGRESS)?
            } else {
                0
            },
            date: date(memory, base, at(address, GOAL_DATE)?)?,
        };
        output.goal_addresses[ordinal] = address;
        if progress {
            read_sections_into(memory, address, &mut output.active_sections[ordinal])?;
        }
    }
    definitions(
        memory,
        base,
        array(memory, status, COMPLETED, 8)?,
        &mut output.completed,
    )?;
    definitions(
        memory,
        base,
        array(memory, status, CANCELLED, 8)?,
        &mut output.cancelled,
    )?;
    let slots = array(memory, status, EXTRA_SLOTS, 4)?;
    resize(&mut output.extra_slots, slots.count)?;
    for (ordinal, value) in output.extra_slots.iter_mut().enumerate() {
        *value = read(
            memory,
            slots
                .element(ordinal as u32, 4)
                .ok_or("faction goal slot address overflow")?,
        )
        .ok_or("faction goal slot unreadable")?;
    }
    let dates = array(memory, status, TERM_DATES, DATE_STRIDE)?;
    resize(&mut output.term_dates, dates.count)?;
    for (ordinal, value) in output.term_dates.iter_mut().enumerate() {
        *value = date(
            memory,
            base,
            dates
                .element(ordinal as u32, DATE_STRIDE)
                .ok_or("faction goal date address overflow")?,
        )?;
    }
    Ok(())
}

fn read_sections_into(
    memory: &impl Memory,
    address: usize,
    values: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let sections = array(memory, address, GOAL_PROGRESS_STATUS, 4)?;
    resize(values, sections.count)?;
    engine_native_memory::cpdx_array::copy_u32s(
        memory,
        sections,
        values,
        "faction goal active section unreadable",
    )
}

pub fn read_progress_into(
    memory: &impl Memory,
    address: usize,
    goal: &mut Goal,
    sections: &mut Vec<u32>,
) -> Result<(), &'static str> {
    goal.progress_raw = word(memory, address, GOAL_PROGRESS)?;
    read_sections_into(memory, address, sections)
}

#[cfg(test)]
mod tests;
