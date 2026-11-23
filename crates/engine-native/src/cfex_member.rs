
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::cship;

pub const VTABLE_RVA: Rva = Rva(0x02a9_a258);
pub const SHIP_HANDLE: usize = 0x08;
pub const CONVOY_HANDLE: usize = 0x10;
pub const OWNER: usize = 0x20;
pub const BUILD_COST_IC: usize = 0x30;
pub const CACHED_STRENGTH: usize = 0x38;
pub const ARRIVAL_HOURS: usize = 0xe4;
pub const RECEIVED_CRITICALS: usize = 0x124;
pub const EVACUATION_STATE: usize = 0x128;
pub const DETECTION_STATE: usize = 0x12c;
pub const ESCAPE_PROGRESS: usize = 0x130;
pub const RECEIVED_DAMAGE: usize = 0x198;

pub const DETAIL_ARENA_CAPACITY: &str = "naval detail nested arena capacity exceeded";

pub enum EntryOutput<'a, T> {
    Owned(&'a mut Vec<T>),
    Slice(&'a mut [T]),
}

const ENTRY_RUN: usize = 64;
const ENTRY_STRIDE: usize = 0x20;
const ENTRY_RECORD: usize = 64;

pub struct EntryRun<'a, M> {
    source: &'a M,
    start: usize,
    bytes: &'a [u8],
}

impl<M: Memory> Memory for EntryRun<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(offset) = address.checked_sub(self.start)
            && offset
                .checked_add(destination.len())
                .is_some_and(|end| end <= self.bytes.len())
        {
            destination.copy_from_slice(&self.bytes[offset..offset + destination.len()]);
            return true;
        }
        self.source.copy(address, destination)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn modifier_definitions(&self) -> Option<&engine_native_memory::modifier_cache::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

pub struct EntryRecord<'a> {
    pointer: usize,
    fields: &'a [engine_native_memory::memory::RecordField],
    record: &'a [u8],
}

impl cship::Served for EntryRecord<'_> {
    fn lookup(&self, address: usize, len: usize) -> cship::Lookup<'_> {
        if self.pointer == 0 || len == 0 || self.record.is_empty() {
            return cship::Lookup::Source;
        }
        let Some(offset) = address.checked_sub(self.pointer) else {
            return cship::Lookup::Source;
        };
        for field in self.fields {
            if offset >= field.source_offset
                && offset
                    .checked_add(len)
                    .is_some_and(|end| end <= field.source_offset + field.len)
            {
                let at = field.destination_offset + offset - field.source_offset;
                return cship::Lookup::Served(&self.record[at..at + len]);
            }
        }
        cship::Lookup::Source
    }
}

pub(crate) type EntryMemory<'a, M> = cship::GatheredMemory<'a, EntryRun<'a, M>, EntryRecord<'a>>;

pub fn append_entries<M: Memory, T>(
    memory: &M,
    array: Array,
    stride: usize,
    mut output: EntryOutput<'_, T>,
    length: &mut usize,
    mut decode: impl FnMut(&EntryMemory<'_, M>, usize) -> Result<T, &'static str>,
    object: Option<&'static [engine_native_memory::memory::RecordField]>,
) -> Result<[usize; 2], &'static str> {
    let count = array.count as usize;
    let end = length
        .checked_add(count)
        .ok_or("naval detail required capacity overflow")?;
    let span = [*length, count];
    match &mut output {
        EntryOutput::Owned(rows) => {
            rows.truncate(*length);
            engine_allocation::reserve_push_batch(rows, count)
                .map_err(|_| "naval detail allocation failed")?;
        }
        EntryOutput::Slice(rows) if end > rows.len() => return Err(DETAIL_ARENA_CAPACITY),
        EntryOutput::Slice(_) => (),
    }
    let fields = object.unwrap_or(&[]);
    let row_bytes = fields
        .iter()
        .map(|field| field.destination_offset + field.len)
        .max()
        .unwrap_or(0);
    let layout = (row_bytes != 0 && row_bytes <= ENTRY_RECORD)
        .then(|| engine_native_memory::memory::RecordLayout::new(fields, row_bytes))
        .flatten();
    let mut elements = [0_u8; ENTRY_RUN * ENTRY_STRIDE];
    let mut records = [0_u8; ENTRY_RUN * ENTRY_RECORD];
    let mut pointers = [0_usize; ENTRY_RUN];
    let mut first = 0_u32;
    while first < array.count {
        let run = ((array.count - first) as usize).min(ENTRY_RUN);
        let start = array
            .element(first, stride)
            .ok_or("naval detail nested element overflow")?;
        let copied = stride <= ENTRY_STRIDE && memory.copy(start, &mut elements[..run * stride]);
        let elements = EntryRun {
            source: memory,
            start,
            bytes: if copied {
                &elements[..run * stride]
            } else {
                &[]
            },
        };
        let mut complete = 0;
        if copied
            && object.is_some()
            && let Some(layout) = &layout
        {
            for (index, pointer) in pointers[..run].iter_mut().enumerate() {
                let at = index * stride;
                *pointer = u64::from_le_bytes(
                    elements.bytes[at..at + 8]
                        .try_into()
                        .expect("pointer element"),
                ) as usize;
            }
            complete = memory.copy_records(
                engine_native_memory::memory::RecordSources::Pointers(&pointers[..run]),
                layout,
                &mut records[..run * row_bytes],
            ) / fields.len().max(1);
        }
        if !copied || (object.is_some() && complete < run) {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::NavalEntryRuns);
        }
        for offset in 0..run {
            let index = first + offset as u32;
            let entry = array
                .element(index, stride)
                .ok_or("naval detail nested element overflow")?;
            let entry = if object.is_some() {
                read::<u64>(&elements, entry).ok_or("naval detail nested pointer unreadable")?
                    as usize
            } else {
                entry
            };
            let record = EntryRecord {
                pointer: entry,
                fields,
                record: if offset < complete {
                    &records[offset * row_bytes..(offset + 1) * row_bytes]
                } else {
                    &[]
                },
            };
            let row = decode(&cship::GatheredMemory::new(&elements, &record), entry)?;
            match &mut output {
                EntryOutput::Owned(rows) => rows.push(row),
                EntryOutput::Slice(rows) => rows[*length + index as usize] = row,
            }
        }
        first += run as u32;
    }
    *length = end;
    Ok(span)
}

pub const STATE: usize = 0xe0;
pub const GROUP: usize = 0xe8;
pub const ATTRIBUTION_NAME: usize = 0xa8;
pub const DAMAGE_ENTRIES: usize = 0x108;
pub const BATTLE_SLOT: usize = 0x120;

#[cfg(test)]
mod serial_reference;

#[cfg(test)]
mod entry_run_tests;
