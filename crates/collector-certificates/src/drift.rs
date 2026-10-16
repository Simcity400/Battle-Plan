//! A drifting country's research and technology rows at its slice, copied
//! from its last complete read with only the daily drift read afresh.
//!
//! Between two reads of a country no hooked writer marked, in hours whose Take certified every
//! write  ([`crate::slicing::Changed`]), its research and technology rows change only by daily
//! drift. For a country owning a state, the daily technology update progresses each slot, banking
//! idle Days  or adding assigned technology progress. The progress adder's other callers run inside
//! hooked writes (assignment, console), reach hooked writes later in the same call (the technology
//! tree sets the level and completes), or are effects its own detour marks
//! (`engine::country_input_mutations::EFFECT_PROGRESS_SOURCE`), so a carried roster reads the
//! progress of the technologies assigned to its slots, and nothing else. A slot's assignment is
//! itself certified (`technology assignment` and `research slot synchronization` mark their
//! country), so the assigned rows are held with the roster, taken from the same pass's
//! ResearchSlots rows. Every other recorded field is the one its last read held, which the family's
//! sampled comparisons check ([`crate::sampled_verify::Check::covers`]).
//!
//! A carry gathers only operands the full walk reads. A failed gather
//! permits one counted serial pass to locate a persistent failure: the
//! country's status through `ctechnology_status::copy_status`/`decode_status`,
//! the roster or slot roster header, then the drift fields. A successful
//! serial fallback publishes its rows under the relaxed read-order rule;
//! a status or roster differing from the held read makes the country read
//! whole. Rows are held only
//! for a country owning a state (the only ones whose slice reads them), and
//! only from a complete read. Storage is advisory and completely fallible:
//! an entry that cannot be held whole is dropped, and the country is read
//! whole at its next slice.
use engine::{
    Memory, cpdx_array::Array, cresearch_slot, ctechnology, ctechnology_status, read,
    research_inputs, research_modifiers,
};
use std::collections::HashMap;

/// One country's last complete technology roster read.
struct Technologies {
    country: usize,
    status: usize,
    roster: Array,
    rows: Vec<ctechnology::Row>,
    pointers: Vec<usize>,
    /// The rows of the technologies assigned to research slots, ascending.
    assigned: Vec<usize>,
    assigned_pointers: Vec<usize>,
}

/// One country's last complete research read.
struct Research {
    country: usize,
    status: usize,
    slots: Array,
    /// Each slot in roster order, with the input row (sorted by slot index)
    /// that it fills.
    slots_to_rows: Vec<(usize, usize)>,
    slot_pointers: Vec<usize>,
    assigned_pointers: Vec<usize>,
    inputs: Vec<research_inputs::Row>,
    bonuses: Vec<research_inputs::BonusRow>,
    modifiers: Vec<research_modifiers::Row>,
}

/// What a carry made of a country's slice.
pub enum Carried<T> {
    /// Its rows from the held read, with the drift read afresh.
    Rows(T),
    /// Nothing usable is held: read it whole.
    Miss,
    /// A live read failed: the country's rows are unavailable this hour,
    /// with the full walk's reason, and nothing is read again.
    Failed(&'static str),
}

/// The held reads of one sliced family, keyed by native country id.
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

    /// Keep only the owners a complete census saw (ascending).
    pub fn retain(&mut self, seen: &[u32]) {
        self.technologies
            .retain(|owner, _| seen.binary_search(owner).is_ok());
        self.research
            .retain(|owner, _| seen.binary_search(owner).is_ok());
    }

    /// Hold nothing for `owner`: its next slice reads it whole.
    pub fn forget(&mut self, owner: u32) {
        self.technologies.remove(&owner);
        self.research.remove(&owner);
    }

    /// Hold a complete technology roster read of `owner` (`country`, the
    /// census object) with its complete ResearchSlots rows of the same pass
    /// (`slots`, `None` when they did not read), or nothing when it cannot
    /// be held whole. It re-reads only the status and roster header the
    /// walk read first.
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
        let held = engine::allocation::advisory(|| {
            let slots = slots?;
            let status = status(memory, base, country).ok()?;
            let mut assigned = Vec::new();
            engine::allocation::reserve(&mut assigned, slots.len()).ok()?;
            for slot in slots.iter().filter(|slot| slot.technology_present != 0) {
                assigned.push(
                    rows.iter()
                        .position(|row| row.database_index == slot.database_index)?,
                );
            }
            // Two slots can research one technology (the loader
            // admits it); the full walk reads each roster row's progress once.
            assigned.sort_unstable();
            assigned.dedup();
            let mut assigned_pointers = Vec::new();
            engine::allocation::reserve_exact(&mut assigned_pointers, assigned.len()).ok()?;
            assigned_pointers.extend(assigned.iter().map(|&row| pointers[row]));
            let held = Technologies {
                country,
                status,
                roster: roster(memory, status)?,
                rows: copied(rows)?,
                pointers: copied(pointers)?,
                assigned,
                assigned_pointers,
            };
            (technologies.contains_key(&owner)
                || engine::allocation::reserve_map(technologies, 1).is_ok())
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

    /// `owner`'s held roster into `rows` and `pointers` from `start`, with
    /// the progress of each technology assigned to a research slot read
    /// afresh, in roster order as the full walk reads it.
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
            if engine::allocation::reserve(rows, end - rows.len()).is_err()
                || engine::allocation::reserve(pointers, end - pointers.len()).is_err()
            {
                return Carried::Failed("technology row allocation failed");
            }
            rows.resize(end, ctechnology::Row::default());
            pointers.resize(end, 0);
        }
        rows[start..end].copy_from_slice(&held.rows);
        pointers[start..end].copy_from_slice(&held.pointers);
        match drift(memory, &held.assigned_pointers, ctechnology::LIVE_PROGRESS) {
            Drift::Copied(progress) => {
                for (&row, progress) in held.assigned.iter().zip(progress) {
                    rows[start + row].progress_raw = progress;
                }
                return Carried::Rows(len);
            }
            Drift::Failed => {
                engine::batch_fallback::record(
                    engine::batch_fallback::Batch::GovernanceTechnologyDrift,
                );
            }
            Drift::Serial => {}
        }
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

    /// Hold a complete research read of `owner` (`country`, the census
    /// object), or nothing when it cannot be held whole. It re-reads only
    /// operands the walk read first: the status, the slot roster and each
    /// slot's index.
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
        let held = engine::allocation::advisory(|| {
            let status = status(memory, base, country).ok()?;
            let slots = slots(memory, status)?;
            // `research_inputs::walk_country_buffer` visits the slots in roster order and sorts its
            // rows by each slot's own index; each slot keeps the row it fills.
            if inputs.len() != slots.count as usize {
                return None;
            }
            let mut slots_to_rows = Vec::new();
            engine::allocation::reserve(&mut slots_to_rows, inputs.len()).ok()?;
            for position in 0..slots.count {
                let slot = read::<u64>(memory, slots.element(position, 8)?)?;
                let slot = usize::try_from(slot).ok().filter(|&slot| slot != 0)?;
                let index = read::<u32>(memory, slot.checked_add(cresearch_slot::INDEX)?)?;
                let row = inputs.iter().position(|row| row.slot.index == index)?;
                slots_to_rows.push((slot, row));
            }
            let mut slot_pointers = Vec::new();
            engine::allocation::reserve_exact(&mut slot_pointers, slots_to_rows.len()).ok()?;
            slot_pointers.extend(slots_to_rows.iter().map(|&(slot, _)| slot));
            let mut assigned_pointers = Vec::new();
            engine::allocation::reserve_exact(&mut assigned_pointers, slots_to_rows.len()).ok()?;
            assigned_pointers.extend(
                slots_to_rows
                    .iter()
                    .map(|&(_, row)| inputs[row].technology_pointer)
                    .filter(|&pointer| pointer != 0),
            );
            let held = Research {
                country,
                status,
                slots,
                slots_to_rows,
                slot_pointers,
                assigned_pointers,
                inputs: copied(inputs)?,
                bonuses: copied(bonuses)?,
                modifiers: copied(modifiers)?,
            };
            (research.contains_key(&owner) || engine::allocation::reserve_map(research, 1).is_ok())
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

    /// `owner`'s held research rows into the three buffers, with each slot's
    /// banked days and its technology's progress read afresh, slot by slot
    /// in roster order as the full walk reads them.
    #[allow(clippy::too_many_arguments)]
    pub fn carry_research(
        &self,
        memory: &impl Memory,
        base: usize,
        owner: u32,
        country: usize,
        mut inputs: engine::walk::Buffer<'_, research_inputs::Row>,
        mut bonuses: engine::walk::Buffer<'_, research_inputs::BonusRow>,
        mut modifiers: engine::walk::Buffer<'_, research_modifiers::Row>,
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
        let progress = if matches!(banked, Drift::Failed) {
            Drift::Serial
        } else {
            drift(memory, &held.assigned_pointers, ctechnology::LIVE_PROGRESS)
        };
        if let (Drift::Copied(banked), Drift::Copied(progress)) = (&banked, &progress) {
            let mut assigned = progress.iter();
            for (&(_, row), &banked) in held.slots_to_rows.iter().zip(banked) {
                inputs[row].slot.banked_days_raw = banked;
                if held.inputs[row].technology_pointer != 0 {
                    inputs[row].progress_raw = *assigned.next().unwrap();
                }
            }
            bonuses[..held.bonuses.len()].clone_from_slice(&held.bonuses);
            modifiers[..held.modifiers.len()].clone_from_slice(&held.modifiers);
            return Carried::Rows([held.inputs.len(), held.bonuses.len(), held.modifiers.len()]);
        }
        if matches!(banked, Drift::Failed) || matches!(progress, Drift::Failed) {
            engine::batch_fallback::record(engine::batch_fallback::Batch::GovernanceResearchDrift);
        }
        // One original serial pass locates persistent failures in slot order.
        // Under the owner's relaxed read-order rule, publish a successful
        // fallback: a fault met only by the gather must not withhold more
        // than the original serial walk. Failed gather bytes are never used.
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

/// The country's authenticated `CTechnologyStatus*`, with the full walks'
/// reasons ([`ctechnology_status::copy_status`], `decode_status`).
fn status(memory: &impl Memory, base: usize, country: usize) -> Result<usize, &'static str> {
    let raw = ctechnology_status::copy_status(memory, country)?;
    ctechnology_status::decode_status(base, country, &raw)
}

/// The status's all-technology roster header.
fn roster(memory: &impl Memory, status: usize) -> Option<Array> {
    Array::read(
        memory,
        status.checked_add(ctechnology_status::TECHNOLOGIES_DATA)?,
        8,
    )
}

/// The status's research slot roster header.
fn slots(memory: &impl Memory, status: usize) -> Option<Array> {
    Array::read(
        memory,
        status.checked_add(ctechnology_status::RESEARCH_SLOTS_DATA)?,
        8,
    )
}

/// A held copy of `rows`, or `None` when storage cannot be had. Called
/// inside [`engine::allocation::advisory`], where a failed text clone yields
/// an empty payload instead of stopping the capture: a copy that does not
/// equal its source is dropped.
fn copied<T: Clone + PartialEq>(rows: &[T]) -> Option<Vec<T>> {
    let mut held = Vec::new();
    engine::allocation::reserve(&mut held, rows.len()).ok()?;
    held.extend(rows.iter().cloned());
    (held == rows).then_some(held)
}

/// A failed gather supplies no values. Advisory storage failure leaves the
/// original serial read; a live fault permits one counted serial fallback.
enum Drift {
    Copied(Vec<i64>),
    Failed,
    Serial,
}
fn drift(memory: &impl Memory, pointers: &[usize], offset: usize) -> Drift {
    if pointers.is_empty() {
        return Drift::Copied(Vec::new());
    }
    let storage = engine::allocation::advisory(|| {
        let len = pointers.len().checked_mul(8)?;
        let mut bytes = Vec::new();
        engine::allocation::reserve_exact(&mut bytes, len).ok()?;
        bytes.resize(len, 0);
        let mut values = Vec::new();
        engine::allocation::reserve_exact(&mut values, pointers.len()).ok()?;
        Some((bytes, values))
    });
    let Some((mut bytes, mut values)) = storage else {
        return Drift::Serial;
    };
    // Daily research slot progress banks into the slot; the technology progress adder technology
    // research progress adder writes the technology's progress. One record contains just that
    // object's qword. Certificates prove these pointers' lifetimes, not readability.
    let fields = [engine::memory::RecordField {
        source_offset: offset,
        destination_offset: 0,
        len: 8,
    }];
    let layout = engine::memory::RecordLayout::new(&fields, 8).unwrap();
    if memory.copy_records(
        engine::memory::RecordSources::Pointers(pointers),
        &layout,
        &mut bytes,
    ) != pointers.len()
    {
        return Drift::Failed;
    }
    values.extend(
        bytes
            .chunks_exact(8)
            .map(|bytes| i64::from_le_bytes(bytes.try_into().unwrap())),
    );
    Drift::Copied(values)
}
