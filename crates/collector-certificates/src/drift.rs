use engine_native::{Memory, cpdx_array::Array, read};
use engine_research::{
    cresearch_slot, ctechnology, ctechnology_status, research_inputs, research_modifiers,
};
use std::collections::HashMap;

#[derive(Default)]
struct Technologies {
    country: usize,
    status: usize,
    roster: Array,
    rows: Vec<ctechnology::Row>,
    pointers: Vec<usize>,
    assigned: Vec<usize>,
    assigned_pointers: Vec<usize>,
}

#[derive(Default)]
struct Research {
    country: usize,
    status: usize,
    slots: Array,
    slots_to_rows: Vec<(usize, usize)>,
    slot_pointers: Vec<usize>,
    assigned_pointers: Vec<usize>,
    slot_assigned: Vec<Option<usize>>,
    first_slot: Vec<(usize, usize)>,
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
        let mut held = self.technologies.remove(&owner).unwrap_or_default();
        let technologies = &mut self.technologies;
        let kept = engine_allocation::advisory(|| {
            let slots = slots?;
            let status = status(memory, base, country).ok()?;
            held.assigned.clear();
            engine_allocation::reserve(&mut held.assigned, slots.len()).ok()?;
            for slot in slots.iter().filter(|slot| slot.technology_present != 0) {
                held.assigned.push(
                    rows.iter()
                        .position(|row| row.database_index == slot.database_index)?,
                );
            }
            held.assigned.sort_unstable();
            held.assigned.dedup();
            held.assigned_pointers.clear();
            engine_allocation::reserve_exact(&mut held.assigned_pointers, held.assigned.len())
                .ok()?;
            held.assigned_pointers
                .extend(held.assigned.iter().map(|&row| pointers[row]));
            held.country = country;
            held.status = status;
            held.roster = roster(memory, status)?;
            copy_into(&mut held.rows, rows)?;
            copy_into(&mut held.pointers, pointers)?;
            engine_allocation::reserve_map(technologies, 1).ok()
        });
        if kept.is_some() {
            self.technologies.insert(owner, held);
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
        let progress = drift(memory, &held.assigned_pointers, ctechnology::LIVE_PROGRESS);
        if matches!(progress, Drift::Partial(_)) {
            engine_native::batch_fallback::record(
                engine_native::batch_fallback::Batch::GovernanceTechnologyDrift,
            );
        }
        let progress = progress.values();
        for (index, &row) in held.assigned.iter().enumerate() {
            let value = if let Some(&value) = progress.get(index) {
                value
            } else {
                let Some(value) = held.pointers[row]
                    .checked_add(ctechnology::LIVE_PROGRESS)
                    .and_then(|at| read::<i64>(memory, at))
                else {
                    return Carried::Failed("technology fields unreadable");
                };
                value
            };
            rows[start + row].progress_raw = value;
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
        let mut held = self.research.remove(&owner).unwrap_or_default();
        let research = &mut self.research;
        let kept = engine_allocation::advisory(|| {
            let status = status(memory, base, country).ok()?;
            let slots = slots(memory, status)?;
            if inputs.len() != slots.count as usize {
                return None;
            }
            held.slots_to_rows.clear();
            engine_allocation::reserve(&mut held.slots_to_rows, inputs.len()).ok()?;
            for position in 0..slots.count {
                let slot = read::<u64>(memory, slots.element(position, 8)?)?;
                let slot = usize::try_from(slot).ok().filter(|&slot| slot != 0)?;
                let index = read::<u32>(memory, slot.checked_add(cresearch_slot::INDEX)?)?;
                let row = inputs.iter().position(|row| row.slot.index == index)?;
                held.slots_to_rows.push((slot, row));
            }
            let len = held.slots_to_rows.len();
            held.slot_pointers.clear();
            engine_allocation::reserve_exact(&mut held.slot_pointers, len).ok()?;
            held.slot_pointers
                .extend(held.slots_to_rows.iter().map(|&(slot, _)| slot));
            held.assigned_pointers.clear();
            engine_allocation::reserve_exact(&mut held.assigned_pointers, len).ok()?;
            held.slot_assigned.clear();
            engine_allocation::reserve_exact(&mut held.slot_assigned, len).ok()?;
            held.first_slot.clear();
            engine_allocation::reserve_exact(&mut held.first_slot, len).ok()?;
            for (slot_index, &(_, row)) in held.slots_to_rows.iter().enumerate() {
                let pointer = inputs[row].technology_pointer;
                let assigned = if pointer == 0 {
                    None
                } else if let Some(existing) =
                    held.assigned_pointers.iter().position(|&at| at == pointer)
                {
                    Some(existing)
                } else {
                    let index = held.assigned_pointers.len();
                    held.assigned_pointers.push(pointer);
                    held.first_slot.push((slot_index, row));
                    Some(index)
                };
                held.slot_assigned.push(assigned);
            }
            held.country = country;
            held.status = status;
            held.slots = slots;
            copy_into(&mut held.inputs, inputs)?;
            copy_into(&mut held.bonuses, bonuses)?;
            copy_into(&mut held.modifiers, modifiers)?;
            engine_allocation::reserve_map(research, 1).ok()
        });
        if kept.is_some() {
            self.research.insert(owner, held);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn carry_research(
        &self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        mut inputs: engine_native::walk::Buffer<'_, research_inputs::Row>,
        mut bonuses: engine_native::walk::Buffer<'_, research_inputs::BonusRow>,
        mut modifiers: engine_native::walk::Buffer<'_, research_modifiers::Row>,
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
        let banked = drift(memory, &held.slot_pointers, cresearch_slot::BANKED_DAYS);
        let progress = if matches!(banked, Drift::Partial(_)) {
            Drift::Serial
        } else {
            drift(memory, &held.assigned_pointers, ctechnology::LIVE_PROGRESS)
        };
        if matches!(banked, Drift::Partial(_)) || matches!(progress, Drift::Partial(_)) {
            engine_native::batch_fallback::record(
                engine_native::batch_fallback::Batch::GovernanceResearchDrift,
            );
        }
        let banked = banked.values();
        let progress = progress.values();
        for (index, &(slot, row)) in held.slots_to_rows.iter().enumerate() {
            let banked = if let Some(&value) = banked.get(index) {
                value
            } else {
                let Some(value) = slot
                    .checked_add(cresearch_slot::BANKED_DAYS)
                    .and_then(|at| read::<i64>(memory, at))
                else {
                    return Carried::Failed("research slot fields unreadable");
                };
                value
            };
            inputs[row].slot.banked_days_raw = banked;
            if let Some(assigned) = held.slot_assigned[index] {
                let (first_index, first_row) = held.first_slot[assigned];
                let value = if let Some(&value) = progress.get(assigned) {
                    value
                } else if first_index < index {
                    inputs[first_row].progress_raw
                } else {
                    let technology = held.inputs[row].technology_pointer;
                    let Some(value) = technology
                        .checked_add(ctechnology::LIVE_PROGRESS)
                        .and_then(|at| read::<i64>(memory, at))
                    else {
                        return Carried::Failed("assigned technology unreadable");
                    };
                    value
                };
                inputs[row].progress_raw = value;
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

fn copy_into<T: Clone + PartialEq>(held: &mut Vec<T>, rows: &[T]) -> Option<()> {
    held.clear();
    engine_allocation::reserve(held, rows.len()).ok()?;
    held.extend(rows.iter().cloned());
    (held.as_slice() == rows).then_some(())
}

enum Drift {
    Complete(Vec<i64>),
    Partial(Vec<i64>),
    Serial,
}
impl Drift {
    fn values(&self) -> &[i64] {
        match self {
            Self::Complete(values) | Self::Partial(values) => values,
            Self::Serial => &[],
        }
    }
}
fn drift(memory: &impl Memory, pointers: &[usize], offset: usize) -> Drift {
    if pointers.is_empty() {
        return Drift::Complete(Vec::new());
    }
    let storage = engine_allocation::advisory(|| {
        let len = pointers.len().checked_mul(8)?;
        let mut bytes = Vec::new();
        engine_allocation::reserve_exact(&mut bytes, len).ok()?;
        bytes.resize(len, 0);
        let mut values = Vec::new();
        engine_allocation::reserve_exact(&mut values, pointers.len()).ok()?;
        Some((bytes, values))
    });
    let Some((mut bytes, mut values)) = storage else {
        return Drift::Serial;
    };
    let fields = [engine_native::memory::RecordField {
        source_offset: offset,
        destination_offset: 0,
        len: 8,
    }];
    let layout = engine_native::memory::RecordLayout::new(&fields, 8).unwrap();
    let done = memory
        .copy_records(
            engine_native::memory::RecordSources::Pointers(pointers),
            &layout,
            &mut bytes,
        )
        .min(pointers.len());
    values.extend(
        bytes[..done * 8]
            .chunks_exact(8)
            .map(|bytes| i64::from_le_bytes(bytes.try_into().unwrap())),
    );
    if done == pointers.len() {
        Drift::Complete(values)
    } else {
        Drift::Partial(values)
    }
}
