use engine_native_parts::cnaval_base_layout as naval_base_layout;

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    read_slice,
};
use crate::{cgame_state, cstate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainCapture {
    pub len: Result<usize, &'static str>,
    pub required: usize,
    pub first_failure: Option<&'static str>,
}

const BATCH: usize = 64;
const NAVAL_BASES: usize = 0x18;
const STATE_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cstate::ID,
        destination_offset: 8,
        len: 4,
    },
];
const STATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_FIELDS, 12).expect("valid launch state layout");
const BASE_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: naval_base_layout::HOLDER,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: naval_base_layout::PROVINCE_ID,
        destination_offset: 16,
        len: 4,
    },
];
const BASE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BASE_FIELDS, 20).expect("valid launch base layout");

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LaunchNavalBase {
    pub base_province: u32,
    pub country: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavalProvider {
    NativeNull,
    Bases(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavalCapture {
    pub required: usize,
    pub provider: Result<NavalProvider, &'static str>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Occurrence {
    pointer: usize,
    ordinal: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReservedOwnedStates {
    root: Option<Result<super::Country, &'static str>>,
    count: Result<i32, &'static str>,
}

impl ReservedOwnedStates {
    pub(super) fn new(memory: &impl Memory, base: usize, state: usize) -> Self {
        let root = super::read_reserved_root(memory, base, state);
        let count = root.and_then(|country| {
            country
                .pointer
                .checked_add(super::OWNED_STATE_COUNT)
                .ok_or("reserved owned state count address overflow")
                .and_then(|address| {
                    read::<i32>(memory, address).ok_or("reserved owned state count unreadable")
                })
        });
        Self {
            root: Some(root),
            count,
        }
    }

    pub(super) fn root(&self) -> Result<super::Country, &'static str> {
        self.root.ok_or("reserved launch state control consumed")?
    }

    pub(super) fn count(&self) -> Result<i32, &'static str> {
        self.count
    }

    pub fn capture(
        self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        scratch: &mut Vec<Occurrence>,
        output: &mut Vec<u32>,
    ) -> DomainCapture {
        self.capture_buffer(
            memory,
            base,
            state,
            engine_native_memory::walk::Buffer::Growing(scratch),
            engine_native_memory::walk::Buffer::Growing(output),
        )
    }

    fn capture_buffer(
        mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        mut scratch: engine_native_memory::walk::Buffer<'_, Occurrence>,
        mut output: engine_native_memory::walk::Buffer<'_, u32>,
    ) -> DomainCapture {
        let root = self
            .root
            .take()
            .ok_or("reserved launch state control consumed")
            .and_then(|root| root);
        let mut required = 0;
        let result = (|| {
            let country = root?;
            let count = self.count?;
            if count == 0 {
                return Ok(0);
            }
            if count < 0 {
                return Err("reserved launch state count is negative");
            }
            required = count as usize;
            let descriptor = country
                .pointer
                .checked_add(super::OWNED_STATES)
                .and_then(|address| read::<[u8; 12]>(memory, address))
                .ok_or("reserved launch state descriptor unreadable")?;
            let mut header = [0; 16];
            header[..12].copy_from_slice(&descriptor);
            header[12..].copy_from_slice(&count.to_le_bytes());
            let source =
                Array::from_header(header, 8).ok_or("reserved launch state descriptor invalid")?;
            scratch.ensure(required)?;
            output.ensure(required)?;
            if output.len() < required || scratch.len() < required {
                return Err("reserved launch state arena exhausted");
            }
            let states = state
                .checked_add(cgame_state::STATES)
                .and_then(|address| Array::read(memory, address, 8))
                .ok_or("reserved launch selecting state array unreadable or invalid")?;
            copy_roster(memory, source, &mut scratch[..required])?;
            capture_state_bodies(
                memory,
                base,
                states,
                &scratch[..required],
                &mut output[..required],
            )?;
            Ok(required)
        })();
        let touched = required.min(scratch.len());
        scratch[..touched].fill(Occurrence::default());
        if result.is_err()
            && let Some(output) = output.get_mut(..required)
        {
            output.fill(0);
        }
        domain(required, result)
    }
}

impl Drop for ReservedOwnedStates {
    fn drop(&mut self) {
        self.root = None;
    }
}

fn domain(required: usize, len: Result<usize, &'static str>) -> DomainCapture {
    DomainCapture {
        required,
        len,
        first_failure: None,
    }
}

fn copy_roster(
    memory: &impl Memory,
    source: Array,
    scratch: &mut [Occurrence],
) -> Result<(), &'static str> {
    let mut bytes = [0; BATCH * 8];
    for (batch, destination) in scratch.chunks_mut(BATCH).enumerate() {
        let first = batch * BATCH;
        let address = source
            .element(first as u32, 8)
            .ok_or("reserved launch pointer array address overflow")?;
        if !read_slice(memory, address, &mut bytes[..destination.len() * 8]) {
            return Err("reserved launch pointer array unreadable");
        }
        for (index, item) in destination.iter_mut().enumerate() {
            let pointer = usize::try_from(u64::from_le_bytes(
                bytes[index * 8..index * 8 + 8].try_into().unwrap(),
            ))
            .map_err(|_| "reserved launch object pointer overflow")?;
            if pointer == 0 {
                return Err("reserved launch object is null");
            }
            *item = Occurrence {
                pointer,
                ordinal: first + index,
            };
        }
    }
    scratch.sort_unstable_by_key(|item| item.pointer);
    Ok(())
}

fn groups(
    scratch: &[Occurrence],
    next: &mut usize,
    pointers: &mut [usize; BATCH],
    ends: &mut [usize; BATCH],
) -> usize {
    let mut len = 0;
    while *next < scratch.len() && len < BATCH {
        let pointer = scratch[*next].pointer;
        pointers[len] = pointer;
        while *next < scratch.len() && scratch[*next].pointer == pointer {
            *next += 1;
        }
        ends[len] = *next;
        len += 1;
    }
    len
}

fn capture_state_bodies(
    memory: &impl Memory,
    base: usize,
    states: Array,
    scratch: &[Occurrence],
    output: &mut [u32],
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, cstate::VTABLE_RVA)
        .ok_or("reserved launch module address overflow")? as u64;
    let mut next = 0;
    while next < scratch.len() {
        let mut first = next;
        let mut pointers = [0; BATCH];
        let mut ends = [0; BATCH];
        let count = groups(scratch, &mut next, &mut pointers, &mut ends);
        let mut bytes = [0; BATCH * 12];
        if memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &STATE_LAYOUT,
            &mut bytes[..count * 12],
        ) != count * STATE_FIELDS.len()
        {
            return Err("reserved launch state body unreadable");
        }
        let mut addresses = [0; BATCH];
        for (index, record) in bytes[..count * 12].chunks_exact(12).enumerate() {
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected {
                return Err("reserved launch state identity mismatch");
            }
            let id = u32::from_le_bytes(record[8..12].try_into().unwrap());
            addresses[index] = states
                .element(id, 8)
                .ok_or("reserved launch state ID outside selecting array")?;
        }
        let mut selected = [[0; 8]; BATCH];
        let mut destinations = selected.iter_mut().enumerate();
        if memory.copy_scattered(
            addresses,
            &mut std::array::from_fn(|_| {
                let (index, bytes) = destinations.next().unwrap();
                &mut bytes[..if index < count { 8 } else { 0 }]
            }),
        ) < count
        {
            return Err("reserved launch selecting state pointer unreadable");
        }
        for index in 0..count {
            if u64::from_le_bytes(selected[index]) != pointers[index] as u64 {
                return Err("reserved launch selecting state identity mismatch");
            }
            let id = u32::from_le_bytes(bytes[index * 12 + 8..index * 12 + 12].try_into().unwrap());
            for item in &scratch[first..ends[index]] {
                output[item.ordinal] = id;
            }
            first = ends[index];
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn capture_naval(
    memory: &impl Memory,
    base: usize,
    navy: usize,
    scratch: &mut Vec<Occurrence>,
    output: &mut Vec<LaunchNavalBase>,
) -> NavalCapture {
    capture_naval_buffer(
        memory,
        base,
        navy,
        engine_native_memory::walk::Buffer::Growing(scratch),
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn capture_naval_buffer(
    memory: &impl Memory,
    base: usize,
    navy: usize,
    mut scratch: engine_native_memory::walk::Buffer<'_, Occurrence>,
    mut output: engine_native_memory::walk::Buffer<'_, LaunchNavalBase>,
) -> NavalCapture {
    if navy == 0 {
        return NavalCapture {
            required: 0,
            provider: Ok(NavalProvider::NativeNull),
        };
    }
    let mut required = 0;
    let result = (|| {
        let source = navy
            .checked_add(NAVAL_BASES)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("reserved launch naval base header unreadable or invalid")?;
        required = source.count as usize;
        if required == 0 {
            return Ok(0);
        }
        scratch.ensure(required)?;
        output.ensure(required)?;
        if output.len() < required || scratch.len() < required {
            return Err("reserved launch naval base arena exhausted");
        }
        let country = navy
            .checked_add(naval_base_layout::HOLDER_COUNTRY)
            .and_then(|address| read::<u32>(memory, address))
            .ok_or("reserved launch naval holder country unreadable")?;
        copy_roster(memory, source, &mut scratch[..required])?;
        let expected = engine_sites::at(base, naval_base_layout::VTABLE_RVA)
            .ok_or("reserved launch module address overflow")? as u64;
        let mut next = 0;
        while next < required {
            let mut first = next;
            let mut pointers = [0; BATCH];
            let mut ends = [0; BATCH];
            let count = groups(&scratch[..required], &mut next, &mut pointers, &mut ends);
            let mut bytes = [0; BATCH * 20];
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &BASE_LAYOUT,
                &mut bytes[..count * 20],
            ) != count * BASE_FIELDS.len()
            {
                return Err("reserved launch naval base body unreadable");
            }
            for (index, record) in bytes[..count * 20].chunks_exact(20).enumerate() {
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected
                    || u64::from_le_bytes(record[8..16].try_into().unwrap()) != navy as u64
                {
                    return Err("reserved launch naval base identity or holder mismatch");
                }
                let value = LaunchNavalBase {
                    base_province: u32::from_le_bytes(record[16..20].try_into().unwrap()),
                    country,
                };
                for item in &scratch[first..ends[index]] {
                    output[item.ordinal] = value;
                }
                first = ends[index];
            }
        }
        Ok(required)
    })();
    let touched = required.min(scratch.len());
    scratch[..touched].fill(Occurrence::default());
    if result.is_err()
        && let Some(output) = output.get_mut(..required)
    {
        output.fill(LaunchNavalBase::default());
    }
    NavalCapture {
        required,
        provider: result.map(NavalProvider::Bases),
    }
}

#[cfg(test)]
mod tests;
