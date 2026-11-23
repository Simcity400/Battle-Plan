
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Buffer,
};
use crate::{cname_group_member::{Names, Span}, cunit};
use engine_native_parts::creference_registry::ScopedResolver;

const SEARCHING: usize = 0x610;
const ENGAGEMENT_TARGET: usize = super::NAVAL_MISSION + 0xa0;
const AVOIDED_SHIP_TYPES: usize = 0x740;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Searcher {
    pub reference: Option<u64>,
    pub resolves: Option<bool>,
    pub roster_owner: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EngagementTarget {
    pub reference: u64,
    pub resolves: Option<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub sample_hour: i32,
    pub searching: Option<Span>,
    pub engagement_target: Option<EngagementTarget>,
    pub avoided_ship_types: Option<Span>,
}

pub struct Arenas<'a> {
    pub searchers: Buffer<'a, Searcher>,
    pub searching_len: usize,
    pub searching_required: usize,
    pub avoided: Buffer<'a, Option<Span>>,
    pub avoided_len: usize,
    pub avoided_required: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReferenceAnswer {
    handle: u64,
    resolution: Option<Result<Option<usize>, &'static str>>,
    owner: Option<Result<Option<u32>, &'static str>>,
}

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

pub struct Collector<'a, M: Memory> {
    reader: Reader<'a, M>,
    arenas: Arenas<'a>,
    names: Names<'a>,
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
            names: Names {
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
        self.reader.capture(
            task,
            mission_kind,
            prefix,
            self.hour,
            &mut self.arenas,
            &mut self.names,
        )
    }

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
    pub fn new(memory: &'a M, base: usize, answers: &'a mut Vec<ReferenceAnswer>) -> Self {
        Self::new_buffer(memory, base, Buffer::Growing(answers))
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
            let _ = engine_allocation::oom();
            "port reference scratch length overflow"
        })?;
        if required <= answers.len() && answers.len().is_power_of_two() {
            return Ok(());
        }
        if required == 0 {
            return Ok(());
        }
        let mut grown = engine_allocation::default_vec::<ReferenceAnswer>(required)
            .map_err(|_| "port reference scratch allocation failed")?;
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
            engine_native_parts::crailway_gun::VTABLE_RVA,
            engine_native_parts::crailway_gun::SECONDARY_VTABLE_RVA,
        ),
        _ => return Err("port searcher CUnit class unsupported"),
    };
    let expected = |rva| {
        engine_sites::at(base, rva)
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
    names: &mut Names<'_>,
) -> Option<Span> {
    let source = task
        .checked_add(AVOIDED_SHIP_TYPES)
        .and_then(|at| Array::read(memory, at, engine_native_memory::msvc_string::STRIDE))?;
    let count = source.count as usize;
    arenas.avoided_required = arenas.avoided_required.saturating_add(count);
    let start = arenas.avoided_len;
    let end = start.checked_add(count)?;
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
                let span = crate::cname_group_member::copy_string(
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
                let mut demand = Names {
                    bytes: engine_native_memory::walk::Buffer::Count,
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
            at(super::NAVAL_MISSION + engine_native_parts::cnaval_mission::ENGAGEMENT_RULE)?,
        )
        .ok_or("port sizing engagement rule unreadable")?;
        if rule == 4 {
            return Ok(());
        }
        let avoided = Array::read(memory, at(AVOIDED_SHIP_TYPES)?, engine_native_memory::msvc_string::STRIDE)
            .ok_or("port sizing avoidance header unreadable or invalid")?;
        demand.avoided = avoided.count as usize;
        for index in 0..avoided.count {
            let one = || -> Result<usize, &'static str> {
                let header = avoided
                    .element(index, engine_native_memory::msvc_string::STRIDE)
                    .and_then(|at| read::<[u8; 32]>(memory, at))
                    .ok_or("port sizing avoided string header unreadable")?;
                let mut names = Names {
                    bytes: engine_native_memory::walk::Buffer::Count,
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
