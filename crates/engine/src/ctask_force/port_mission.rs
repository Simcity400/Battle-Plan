//! Stored operands of the political port tooltip (14197C410/14197A6D0).
//!
//! The fleet visitor has already authenticated the primary task-force pointer,
//! its full handle and its embedded mission. No unit selection, hover cache or
//! native function call is acquired. Owned ranges survive the callback; the
//! reference memo's temporary native pointers are cleared on drop.

use crate::{
    Memory,
    cname_group_member::{NameArena, Names, Span},
    cpdx_array::Array,
    creference_registry::ScopedResolver,
    cunit,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Buffer,
};

/// Ordered full searcher CIDs. Add140D50030 appends; remove140D62020 tail-swaps.
/// Save140D66770 (secondary receiver) writes primary610/count61C, token3BAB.
const SEARCHING: usize = 0x610;
/// Mission target+A0. Setter140FA5AE0 and simulation140D48FB0 own it;
/// serialize140FA9900 writes token3BB1 whenever either raw word is nonzero.
const ENGAGEMENT_TARGET: usize = super::NAVAL_MISSION + 0xa0;
/// Ordered 32-byte strings. Command14133CB20 copies them with14011F6B0;
/// save/load140D66770/140D60C40 use token2810 (`target_ship_types`).
const AVOIDED_SHIP_TYPES: usize = 0x740;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Searcher {
    /// None is an unreadable raw slot, not a null or dangling reference.
    pub reference: Option<u64>,
    pub resolves: Option<bool>,
    /// Raw CUnit primary+1D8, including zero; never logical owner+1E0.
    pub roster_owner: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EngagementTarget {
    pub reference: u64,
    /// Exactly the native resolution predicate: secondary is nonnull and not
    /// 0x10. No CTaskForce type/class gate is applied to this liveness bit.
    pub resolves: Option<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub sample_hour: i32,
    /// (start, len) in the caller's Searcher arena; empty is a known empty list.
    pub searching: Option<Span>,
    pub engagement_target: Option<EngagementTarget>,
    /// (start, len) in the caller's nullable byte-span arena.
    pub avoided_ship_types: Option<Span>,
}

/// Storage is prepared off-thread. The shared byte arena lends an independent
/// port partition, so these strings cannot consume cached-name capacity.
pub struct Arenas<'a> {
    pub searchers: Buffer<'a, Searcher>,
    pub searching_len: usize,
    pub searching_required: usize,
    pub avoided: Buffer<'a, Option<Span>>,
    pub avoided_len: usize,
    pub avoided_required: usize,
}

/// This is capture scratch, never a recorded row. Every pointer is erased by
/// Reader::drop before the acquisition boundary returns, including a failed walk.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReferenceAnswer {
    handle: u64,
    resolution: Option<Result<Option<usize>, &'static str>>,
    owner: Option<Result<Option<u32>, &'static str>>,
}

/// Reserve twice the complete occurrence demand, rounded to a power of two.
/// Full probing never evicts or retries a failed observation.
pub fn reference_slots(occurrences: usize) -> Option<usize> {
    if occurrences == 0 {
        Some(0)
    } else {
        occurrences.checked_mul(2)?.checked_next_power_of_two()
    }
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    registry: ScopedResolver,
    answers: Buffer<'a, ReferenceAnswer>,
    pub reference_required: usize,
}

/// The complete port supplement loan for one fleet traversal. Both byte and
/// record cursors are independent from the existing name and supply loans.
pub struct Collector<'a, M: Memory> {
    reader: Reader<'a, M>,
    arenas: Arenas<'a>,
    names: NameArena<'a>,
    hour: i32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Captured {
    pub searching_len: usize,
    pub searching_required: usize,
    pub avoided_len: usize,
    pub avoided_required: usize,
    pub bytes_len: usize,
    pub bytes_required: usize,
    pub reference_required: usize,
}

impl<'a, M: Memory> Collector<'a, M> {
    pub fn new(
        memory: &'a M,
        base: usize,
        hour: i32,
        searchers: &'a mut [Searcher],
        avoided: &'a mut [Option<Span>],
        bytes: &'a mut [u8],
        answers: &'a mut [ReferenceAnswer],
    ) -> Self {
        Self::new_buffer(
            memory,
            base,
            hour,
            Buffer::Fixed(searchers),
            Buffer::Fixed(avoided),
            Buffer::Fixed(bytes),
            Buffer::Fixed(answers),
        )
    }

    /// Worker-owned storage grows from the original search/string headers before
    /// publication (ordered native loops140D66770/14197C410).
    pub fn new_growing(
        memory: &'a M,
        base: usize,
        hour: i32,
        searchers: &'a mut Vec<Searcher>,
        avoided: &'a mut Vec<Option<Span>>,
        bytes: &'a mut Vec<u8>,
        answers: &'a mut Vec<ReferenceAnswer>,
    ) -> Self {
        Self::new_buffer(
            memory,
            base,
            hour,
            Buffer::Growing(searchers),
            Buffer::Growing(avoided),
            Buffer::Growing(bytes),
            Buffer::Growing(answers),
        )
    }

    fn new_buffer(
        memory: &'a M,
        base: usize,
        hour: i32,
        searchers: Buffer<'a, Searcher>,
        avoided: Buffer<'a, Option<Span>>,
        bytes: Buffer<'a, u8>,
        answers: Buffer<'a, ReferenceAnswer>,
    ) -> Self {
        Self {
            reader: Reader::new_buffer(memory, base, answers),
            arenas: Arenas {
                searchers,
                searching_len: 0,
                searching_required: 0,
                avoided,
                avoided_len: 0,
                avoided_required: 0,
            },
            names: NameArena {
                bytes,
                len: 0,
                required: 0,
            },
            hour,
        }
    }

    pub fn capture(
        &mut self,
        task: usize,
        mission_kind: u32,
        prefix: &super::CounterRow,
    ) -> Inputs {
        self.reader.capture_buffer(
            task,
            mission_kind,
            prefix,
            self.hour,
            &mut self.arenas,
            &mut self.names,
        )
    }

    /// Finishing or dropping erases all transient reference answers. Only
    /// copied scalars and arena ranges survive the acquisition boundary.
    pub fn finish(self) -> Captured {
        Captured {
            searching_len: self.arenas.searching_len,
            searching_required: self.arenas.searching_required,
            avoided_len: self.arenas.avoided_len,
            avoided_required: self.arenas.avoided_required,
            bytes_len: self.names.len,
            bytes_required: self.names.required,
            reference_required: self.reader.reference_required,
        }
    }
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, answers: &'a mut [ReferenceAnswer]) -> Self {
        Self::new_buffer(memory, base, Buffer::Fixed(answers))
    }

    fn new_buffer(memory: &'a M, base: usize, mut answers: Buffer<'a, ReferenceAnswer>) -> Self {
        answers.fill(ReferenceAnswer::default());
        Self {
            memory,
            base,
            registry: ScopedResolver::new(base),
            answers,
            reference_required: 0,
        }
    }

    fn prepare_answers(&mut self) -> Result<(), &'static str> {
        let Buffer::Growing(answers) = &mut self.answers else {
            return Ok(());
        };
        let required = reference_slots(self.reference_required).ok_or_else(|| {
            let _ = crate::allocation::oom();
            "port reference scratch length overflow"
        })?;
        if required <= answers.len() && answers.len().is_power_of_two() {
            return Ok(());
        }
        if required == 0 {
            return Ok(());
        }
        let mut grown = crate::allocation::default_vec::<ReferenceAnswer>(required)
            .map_err(|_| "port reference scratch allocation failed")?;
        // Grow only owned answers, retaining sticky native faults and owners.
        // No reference is resolved again after the original header's growth.
        for answer in answers.iter().filter(|answer| answer.resolution.is_some()) {
            let hash = answer
                .handle
                .rotate_left(32)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let first = (hash ^ (hash >> 32)) as usize & (required - 1);
            let index = (0..required)
                .map(|probe| first.wrapping_add(probe) & (required - 1))
                .find(|&index| grown[index].resolution.is_none())
                .expect("twice the occurrence demand fits every retained answer");
            grown[index] = *answer;
        }
        **answers = grown;
        Ok(())
    }

    fn answer(&mut self, handle: u64) -> Result<usize, &'static str> {
        if self.answers.is_empty() || !self.answers.len().is_power_of_two() {
            return Err("port reference scratch does not fit");
        }
        let hash = handle.rotate_left(32).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let first = (hash ^ (hash >> 32)) as usize & (self.answers.len() - 1);
        for probe in 0..self.answers.len() {
            let index = first.wrapping_add(probe) & (self.answers.len() - 1);
            let entry = &mut self.answers[index];
            if entry.resolution.is_none() {
                entry.handle = handle;
                entry.resolution = Some(self.registry.resolve(self.memory, handle));
                return Ok(index);
            }
            if entry.handle == handle {
                return Ok(index);
            }
        }
        Err("port reference scratch does not fit")
    }

    fn resolution(&mut self, handle: u64) -> Result<Option<(usize, usize)>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        let index = self.answer(handle)?;
        match self.answers[index].resolution.expect("answer observed")? {
            Some(secondary) if secondary != 0x10 => Ok(Some((index, secondary))),
            _ => Ok(None),
        }
    }

    fn searching_owner(&mut self, reference: u64) -> Searcher {
        let mut observed = Searcher {
            reference: Some(reference),
            ..Searcher::default()
        };
        match self.resolution(reference) {
            Ok(None) => observed.resolves = Some(false),
            Err(_) => {}
            Ok(Some((index, secondary))) => {
                let owner = if let Some(owner) = self.answers[index].owner {
                    owner
                } else {
                    let owner = generic_owner(self.memory, self.base, reference, secondary);
                    self.answers[index].owner = Some(owner);
                    owner
                };
                if let Ok(owner) = owner {
                    observed.resolves = Some(true);
                    observed.roster_owner = owner;
                }
            }
        }
        observed
    }

    pub fn capture(
        &mut self,
        task: usize,
        mission_kind: u32,
        prefix: &super::CounterRow,
        hour: i32,
        arenas: &mut Arenas<'_>,
        names: &mut Names<'_>,
    ) -> Inputs {
        let mut arena = NameArena {
            bytes: Buffer::Fixed(names.bytes),
            len: names.len,
            required: names.required,
        };
        let inputs = self.capture_buffer(task, mission_kind, prefix, hour, arenas, &mut arena);
        names.len = arena.len;
        names.required = arena.required;
        inputs
    }

    fn capture_buffer(
        &mut self,
        task: usize,
        mission_kind: u32,
        prefix: &super::CounterRow,
        hour: i32,
        arenas: &mut Arenas<'_>,
        names: &mut NameArena<'_>,
    ) -> Inputs {
        let mut inputs = Inputs {
            sample_hour: hour,
            ..Inputs::default()
        };
        inputs.searching = self.searching(task, arenas);
        if mission_kind != 2 || prefix.detached_activity != 0 {
            return inputs;
        }
        let combats = task
            .checked_add(cunit::COMBAT_REFERENCE_COUNT)
            .and_then(|at| read::<i32>(self.memory, at));
        if !matches!(combats, Some(count) if count <= 0) {
            return inputs;
        }
        inputs.engagement_target = task
            .checked_add(ENGAGEMENT_TARGET)
            .and_then(|at| read::<u64>(self.memory, at))
            .map(|reference| {
                self.reference_required = self.reference_required.saturating_add(1);
                EngagementTarget {
                    reference,
                    resolves: self
                        .prepare_answers()
                        .and_then(|()| self.resolution(reference))
                        .map(|resolved| resolved.is_some())
                        .ok(),
                }
            });
        // Rule four and a known resolving target bypass readiness/avoidance.
        // A failed target read cannot hide a separately readable late group.
        if prefix.engagement_rule != 4
            && inputs
                .engagement_target
                .is_none_or(|target| target.resolves != Some(true))
        {
            inputs.avoided_ship_types = avoided(self.memory, task, arenas, names);
        }
        inputs
    }

    fn searching(&mut self, task: usize, arenas: &mut Arenas<'_>) -> Option<Span> {
        let source = task
            .checked_add(SEARCHING)
            .and_then(|at| Array::read(self.memory, at, 8))?;
        let count = source.count as usize;
        arenas.searching_required = arenas.searching_required.saturating_add(count);
        self.reference_required = self.reference_required.saturating_add(count);
        let start = arenas.searching_len;
        let end = start.checked_add(count)?;
        // 14197C410 visits the complete original +610/+61C searcher list.
        arenas.searchers.ensure(end).ok()?;
        self.prepare_answers().ok()?;
        let output = arenas.searchers.get_mut(start..end)?;
        output.fill(Searcher::default());
        const BATCH: usize = 64;
        const FIELD: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        let layout = RecordLayout::new(&FIELD, 8).expect("full CID layout");
        let mut bytes = [0; BATCH * 8];
        for first in (0..count).step_by(BATCH) {
            let len = (count - first).min(BATCH);
            let data = source.element(first as u32, 8)?;
            let copied = self
                .memory
                .copy_records(
                    RecordSources::Strided {
                        base: data,
                        stride: 8,
                        count: len,
                    },
                    &layout,
                    &mut bytes[..len * 8],
                )
                .min(len);
            for index in 0..copied {
                let at = index * 8;
                let reference = u64::from_le_bytes(bytes[at..at + 8].try_into().expect("full CID"));
                output[first + index] = self.searching_owner(reference);
            }
            if copied < len {
                break;
            }
        }
        // Unreadable raw slots remain None; the readable ordered prefix is kept.
        arenas.searching_len = end;
        Some((start, count))
    }
}

impl<M: Memory> Drop for Reader<'_, M> {
    fn drop(&mut self) {
        self.answers.fill(ReferenceAnswer::default());
    }
}

fn generic_owner(
    memory: &impl Memory,
    base: usize,
    reference: u64,
    secondary: usize,
) -> Result<Option<u32>, &'static str> {
    let primary = secondary
        .checked_sub(super::SECONDARY_BASE)
        .filter(|pointer| *pointer != 0)
        .ok_or("port searcher primary invalid")?;
    let header = read::<[u8; 32]>(memory, primary).ok_or("port searcher identity unreadable")?;
    if reference >> 32 == 0 {
        return Err("port searcher CUnit identity mismatch");
    }
    let class = u32::from_le_bytes(header[8..12].try_into().expect("class"));
    let (primary_vtable, secondary_vtable) = match class {
        0 => (crate::carmy::VTABLE_RVA, crate::carmy::SECONDARY_VTABLE_RVA),
        1 => (super::VTABLE_RVA, super::SECONDARY_VTABLE_RVA),
        13 => (
            crate::crailway_gun::VTABLE_RVA,
            crate::crailway_gun::SECONDARY_VTABLE_RVA,
        ),
        _ => return Err("port searcher CUnit class unsupported"),
    };
    let expected = |rva| {
        base.checked_add(rva)
            .map(|pointer| pointer as u64)
            .ok_or("port searcher module address overflow")
    };
    if u64::from_le_bytes(header[..8].try_into().expect("primary vtable"))
        != expected(primary_vtable)?
        || u64::from_le_bytes(header[16..24].try_into().expect("secondary vtable"))
            != expected(secondary_vtable)?
        || u64::from_le_bytes(header[24..32].try_into().expect("handle")) != reference
    {
        return Err("port searcher CUnit identity mismatch");
    }
    Ok(primary
        .checked_add(cunit::ROSTER_OWNER)
        .and_then(|at| read(memory, at)))
}

fn avoided(
    memory: &impl Memory,
    task: usize,
    arenas: &mut Arenas<'_>,
    names: &mut NameArena<'_>,
) -> Option<Span> {
    let source = task
        .checked_add(AVOIDED_SHIP_TYPES)
        .and_then(|at| Array::read(memory, at, crate::msvc_string::STRIDE))?;
    let count = source.count as usize;
    arenas.avoided_required = arenas.avoided_required.saturating_add(count);
    let start = arenas.avoided_len;
    let end = start.checked_add(count)?;
    // Save140D66770 retains every ordered string slot.
    arenas.avoided.ensure(end).ok()?;
    let mut output = arenas.avoided.get_mut(start..end);
    if let Some(output) = output.as_deref_mut() {
        output.fill(None);
    }
    const BATCH: usize = 16;
    const FIELD: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 32,
    }];
    let layout = RecordLayout::new(&FIELD, 32).expect("MSVC string layout");
    let mut bytes = [0; BATCH * 32];
    const ERRORS: crate::cname_group_member::StringErrors = [
        "port avoided ship type header invalid",
        "port avoided ship type pointer invalid",
        "port avoided ship type bytes unreadable",
        "port avoided ship type terminator invalid",
    ];
    for first in (0..count).step_by(BATCH) {
        let len = (count - first).min(BATCH);
        let data = source.element(first as u32, 32)?;
        let copied = memory
            .copy_records(
                RecordSources::Strided {
                    base: data,
                    stride: 32,
                    count: len,
                },
                &layout,
                &mut bytes[..len * 32],
            )
            .min(len);
        for index in 0..copied {
            let at = index * 32;
            if let Some(output) = output.as_deref_mut() {
                let span = crate::cname_group_member::copy_string_buffer(
                    memory,
                    &bytes[at..at + 32],
                    names,
                    &ERRORS,
                )
                .ok()
                .flatten();
                output[first + index] = span;
                if let Some((start, len)) = span {
                    names.len = start + len;
                }
            } else {
                // A missing span arena still counts every independently valid
                // byte demand, without copying/committing orphan payloads.
                let mut demand = Names {
                    bytes: &mut [],
                    len: 0,
                    required: 0,
                };
                let _ = crate::cname_group_member::copy_string(
                    memory,
                    &bytes[at..at + 32],
                    &mut demand,
                    &ERRORS,
                );
                names.required = names.required.saturating_add(demand.required);
            }
        }
        if copied < len {
            break;
        }
    }
    output?;
    arenas.avoided_len = end;
    Some((start, count))
}

/// Header-only demand for the same authenticated fleet visitor used by hourly
/// capture. Failed components remain named; they never supply an empty list.
#[derive(Debug, Default, Clone, Copy)]
pub struct Demand {
    pub searchers: usize,
    pub avoided: usize,
    pub bytes: usize,
    pub references: usize,
    pub refusal: Option<&'static str>,
}

pub fn demand(memory: &impl Memory, base: usize, task: usize, mission_kind: u32) -> Demand {
    let mut demand = Demand::default();
    let at = |offset| {
        task.checked_add(offset)
            .ok_or("port sizing task address overflow")
    };
    let searching = at(SEARCHING).ok().and_then(|at| Array::read(memory, at, 8));
    if let Some(searching) = searching {
        demand.searchers = searching.count as usize;
        demand.references = demand.searchers;
    } else {
        demand.refusal = Some("port sizing searching header unreadable or invalid");
    }
    let mut late = || -> Result<(), &'static str> {
        if mission_kind != 2 {
            return Ok(());
        }
        let header = read::<[u8; 16]>(memory, at(super::NAVAL_MISSION)?)
            .ok_or("port sizing naval mission identity unreadable")?;
        super::authenticate_mission(&header, base, task)?;
        let detached = read::<i32>(memory, at(super::DETACHED_ACTIVITY)?)
            .ok_or("port sizing detached activity unreadable")?;
        if detached != 0 {
            return Ok(());
        }
        let combats = read::<i32>(memory, at(cunit::COMBAT_REFERENCE_COUNT)?)
            .ok_or("port sizing combat count unreadable")?;
        if combats > 0 {
            return Ok(());
        }
        demand.references = demand.references.saturating_add(1);
        let rule = read::<i32>(
            memory,
            at(super::NAVAL_MISSION + crate::cnaval_mission::ENGAGEMENT_RULE)?,
        )
        .ok_or("port sizing engagement rule unreadable")?;
        if rule == 4 {
            return Ok(());
        }
        // Target liveness can skip this late group in capture. Conservatively
        // prepare its real native demand without adding a registry census here.
        let avoided = Array::read(memory, at(AVOIDED_SHIP_TYPES)?, crate::msvc_string::STRIDE)
            .ok_or("port sizing avoidance header unreadable or invalid")?;
        demand.avoided = avoided.count as usize;
        for index in 0..avoided.count {
            let one = || -> Result<usize, &'static str> {
                let header = avoided
                    .element(index, crate::msvc_string::STRIDE)
                    .and_then(|at| read::<[u8; 32]>(memory, at))
                    .ok_or("port sizing avoided string header unreadable")?;
                let mut names = Names {
                    bytes: &mut [],
                    len: 0,
                    required: 0,
                };
                crate::cname_group_member::copy_string(
                    memory,
                    &header,
                    &mut names,
                    &[
                        "port sizing avoided string header invalid",
                        "port sizing avoided string pointer invalid",
                        "port sizing avoided string bytes unreadable",
                        "port sizing avoided string terminator invalid",
                    ],
                )?;
                Ok(names.required)
            };
            match one() {
                Ok(bytes) => {
                    demand.bytes = demand
                        .bytes
                        .checked_add(bytes)
                        .ok_or("port sizing avoided bytes overflow")?
                }
                Err(reason) => {
                    demand.refusal.get_or_insert(reason);
                }
            }
        }
        Ok(())
    };
    if let Err(reason) = late() {
        demand.refusal.get_or_insert(reason);
    }
    demand
}

#[cfg(test)]
mod tests;
