use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_research::{
    cresearch_slot, ctechnology, ctechnology_status, research_inputs, research_modifiers,
};
use std::collections::HashMap;

struct Technologies {
    country: usize,
    status: usize,
    roster: Array,
    rows: Vec<ctechnology::Row>,
    pointers: Vec<usize>,
    assigned: Vec<usize>,
}

struct Research {
    country: usize,
    status: usize,
    slots: Array,
    slots_to_rows: Vec<(usize, usize)>,
    inputs: Vec<research_inputs::Row>,
    bonuses: Vec<research_inputs::BonusRow>,
    modifiers: Vec<research_modifiers::Row>,
}

pub enum Carried<T> {
    Rows(T),
    Miss,
    Failed(&'static str),
}

#[derive(Default)]
pub struct Store {
    technologies: HashMap<u32, Technologies>,
    research: HashMap<u32, Research>,
}

impl Store {
    pub fn clear(&mut self) {
        self.technologies.clear();
        self.research.clear();
    }

    pub fn retain(&mut self, seen: &[u32]) {
        self.technologies
            .retain(|owner, _| seen.binary_search(owner).is_ok());
        self.research
            .retain(|owner, _| seen.binary_search(owner).is_ok());
    }

    pub fn forget(&mut self, owner: u32) {
        self.technologies.remove(&owner);
        self.research.remove(&owner);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn hold_technologies(
        &mut self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        rows: &[ctechnology::Row],
        pointers: &[usize],
        slots: Option<&[cresearch_slot::Row]>,
    ) {
        let technologies = &mut self.technologies;
        let held = engine_allocation::advisory(|| {
            let slots = slots?;
            let status = status(memory, base, country).ok()?;
            let mut assigned = Vec::new();
            engine_allocation::reserve(&mut assigned, slots.len()).ok()?;
            for slot in slots.iter().filter(|slot| slot.technology_present != 0) {
                assigned.push(
                    rows.iter()
                        .position(|row| row.database_index == slot.database_index)?,
                );
            }
            assigned.sort_unstable();
            assigned.dedup();
            let held = Technologies {
                country,
                status,
                roster: roster(memory, status)?,
                rows: copied(rows)?,
                pointers: copied(pointers)?,
                assigned,
            };
            (technologies.contains_key(&owner)
                || engine_allocation::reserve_map(technologies, 1).is_ok())
            .then_some(held)
        });
        match held {
            Some(held) => {
                self.technologies.insert(owner, held);
            }
            None => {
                self.technologies.remove(&owner);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn carry_technologies(
        &self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        rows: &mut Vec<ctechnology::Row>,
        pointers: &mut Vec<usize>,
        start: usize,
    ) -> Carried<usize> {
        let Some(held) = self
            .technologies
            .get(&owner)
            .filter(|held| held.country == country)
        else {
            return Carried::Miss;
        };
        let identity = (|| -> Result<bool, &'static str> {
            let status = status(memory, base, country)?;
            if status != held.status {
                return Ok(false);
            }
            let roster = roster(memory, status).ok_or("technology roster header invalid")?;
            Ok(roster == held.roster)
        })();
        match identity {
            Err(reason) => return Carried::Failed(reason),
            Ok(false) => return Carried::Miss,
            Ok(true) => {}
        }
        let len = held.rows.len();
        let Some(end) = start.checked_add(len) else {
            return Carried::Failed("technology row count overflow");
        };
        if end > rows.len() {
            if engine_allocation::reserve(rows, end - rows.len()).is_err()
                || engine_allocation::reserve(pointers, end - pointers.len()).is_err()
            {
                return Carried::Failed("technology row allocation failed");
            }
            rows.resize(end, ctechnology::Row::default());
            pointers.resize(end, 0);
        }
        rows[start..end].copy_from_slice(&held.rows);
        pointers[start..end].copy_from_slice(&held.pointers);
        for &row in &held.assigned {
            let progress = held.pointers[row]
                .checked_add(ctechnology::LIVE_PROGRESS)
                .and_then(|at| read::<i64>(memory, at));
            match progress {
                Some(progress) => rows[start + row].progress_raw = progress,
                None => return Carried::Failed("technology fields unreadable"),
            }
        }
        Carried::Rows(len)
    }

    pub fn hold_research(
        &mut self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        rows: (
            &[research_inputs::Row],
            &[research_inputs::BonusRow],
            &[research_modifiers::Row],
        ),
    ) {
        let (inputs, bonuses, modifiers) = rows;
        let research = &mut self.research;
        let held = engine_allocation::advisory(|| {
            let status = status(memory, base, country).ok()?;
            let slots = slots(memory, status)?;
            if inputs.len() != slots.count as usize {
                return None;
            }
            let mut slots_to_rows = Vec::new();
            engine_allocation::reserve(&mut slots_to_rows, inputs.len()).ok()?;
            for position in 0..slots.count {
                let slot = read::<u64>(memory, slots.element(position, 8)?)?;
                let slot = usize::try_from(slot).ok().filter(|&slot| slot != 0)?;
                let index = read::<u32>(memory, slot.checked_add(cresearch_slot::INDEX)?)?;
                let row = inputs.iter().position(|row| row.slot.index == index)?;
                slots_to_rows.push((slot, row));
            }
            let held = Research {
                country,
                status,
                slots,
                slots_to_rows,
                inputs: copied(inputs)?,
                bonuses: copied(bonuses)?,
                modifiers: copied(modifiers)?,
            };
            (research.contains_key(&owner) || engine_allocation::reserve_map(research, 1).is_ok())
                .then_some(held)
        });
        match held {
            Some(held) => {
                self.research.insert(owner, held);
            }
            None => {
                self.research.remove(&owner);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn carry_research(
        &self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        mut inputs: engine_native_memory::walk::Buffer<'_, research_inputs::Row>,
        mut bonuses: engine_native_memory::walk::Buffer<'_, research_inputs::BonusRow>,
        mut modifiers: engine_native_memory::walk::Buffer<'_, research_modifiers::Row>,
    ) -> Carried<[usize; 3]> {
        let Some(held) = self
            .research
            .get(&owner)
            .filter(|held| held.country == country)
        else {
            return Carried::Miss;
        };
        let identity = (|| -> Result<bool, &'static str> {
            let status = status(memory, base, country)?;
            if status != held.status {
                return Ok(false);
            }
            let slots = slots(memory, status).ok_or("research slots header invalid")?;
            Ok(slots == held.slots)
        })();
        match identity {
            Err(reason) => return Carried::Failed(reason),
            Ok(false) => return Carried::Miss,
            Ok(true) => {}
        }
        let grown = (|| {
            Ok::<bool, &'static str>(
                inputs.ensure(held.inputs.len())?
                    && bonuses.ensure(held.bonuses.len())?
                    && modifiers.ensure(held.modifiers.len())?,
            )
        })();
        match grown {
            Ok(true) => {}
            Ok(false) => return Carried::Miss,
            Err(reason) => return Carried::Failed(reason),
        }
        inputs[..held.inputs.len()].clone_from_slice(&held.inputs);
        for &(slot, row) in &held.slots_to_rows {
            let Some(banked) = slot
                .checked_add(cresearch_slot::BANKED_DAYS)
                .and_then(|at| read::<i64>(memory, at))
            else {
                return Carried::Failed("research slot fields unreadable");
            };
            inputs[row].slot.banked_days_raw = banked;
            let technology = held.inputs[row].technology_pointer;
            if technology != 0 {
                let Some(progress) = technology
                    .checked_add(ctechnology::LIVE_PROGRESS)
                    .and_then(|at| read::<i64>(memory, at))
                else {
                    return Carried::Failed("assigned technology unreadable");
                };
                inputs[row].progress_raw = progress;
            }
        }
        bonuses[..held.bonuses.len()].clone_from_slice(&held.bonuses);
        modifiers[..held.modifiers.len()].clone_from_slice(&held.modifiers);
        Carried::Rows([held.inputs.len(), held.bonuses.len(), held.modifiers.len()])
    }
}

fn status(memory: &impl Memory, base: usize, country: usize) -> Result<usize, &'static str> {
    let raw = ctechnology_status::copy_status(memory, country)?;
    ctechnology_status::decode_status(base, country, &raw)
}

fn roster(memory: &impl Memory, status: usize) -> Option<Array> {
    Array::read(
        memory,
        status.checked_add(ctechnology_status::TECHNOLOGIES_DATA)?,
        8,
    )
}

fn slots(memory: &impl Memory, status: usize) -> Option<Array> {
    Array::read(
        memory,
        status.checked_add(ctechnology_status::RESEARCH_SLOTS_DATA)?,
        8,
    )
}

fn copied<T: Clone + PartialEq>(rows: &[T]) -> Option<Vec<T>> {
    let mut held = Vec::new();
    engine_allocation::reserve(&mut held, rows.len()).ok()?;
    held.extend(rows.iter().cloned());
    (held == rows).then_some(held)
}
