
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::read,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native::{carmy, ccountry, cfleet, ctask_force, cunit};
use engine_native_parts::crailway_gun;
use engine_sites::Rva;

pub const STRIDE: usize = 24;
const ENTRY_BATCH: usize = 32;
const GROUP: usize = 16;
pub const VIEWER: usize = 0x00;
#[cfg(test)]
pub const TIMER: usize = 0x08;
pub const LEVEL: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub unit: u64,
    pub ordinal: u32,
    pub viewer: u32,
    pub level: i8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionSource {
    pub pointer: usize,
    pub canonical: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DivisionSources<'a> {
    pub entries: &'a [DivisionSource],
    pub outcome: Outcome,
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut Vec<Row>) -> Outcome {
    walk_with_divisions_buffer(memory, base, game_state, Buffer::Growing(output), None)
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_with_divisions_buffer(memory, base, game_state, Buffer::Count, None)
}

pub fn walk_with_divisions_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
    divisions: Option<DivisionSources<'_>>,
) -> Outcome {
    let result = (|| {
        if let Some(divisions) = divisions {
            match divisions.outcome {
                Outcome::Rows { len } if len == divisions.entries.len() => {}
                Outcome::Unavailable { reason } => return Err(reason),
                _ => return Err("shared division census incomplete"),
            }
        }
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let reader = Reader { memory, base };
        let mut counter = Counter {
            len: 0,
            unread_rows: 0,
        };
        let mut next_division = 0;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            if let Some(divisions) = divisions {
                let first = next_division;
                while divisions
                    .entries
                    .get(next_division)
                    .is_some_and(|entry| entry.canonical == country.canonical_slot)
                {
                    next_division += 1;
                }
                if divisions
                    .entries
                    .get(next_division)
                    .is_some_and(|entry| entry.canonical < country.canonical_slot)
                {
                    return Err("shared division census country order invalid");
                }
                reader.divisions(
                    &divisions.entries[first..next_division],
                    &mut output,
                    &mut counter,
                )?;
            } else {
                for unit in reader.members(country.pointer, ccountry::DIVISIONS, DIVISIONS)? {
                    reader.unit(unit?, carmy::VTABLE_RVA, &mut output, &mut counter)?;
                }
            }
            for fleet in reader.members(country.pointer, ccountry::FLEETS, FLEETS)? {
                let fleet = fleet?;
                reader.vtable(fleet, cfleet::VTABLE_RVA, "fleet vtable mismatch")?;
                for unit in reader.members(fleet, cfleet::TASK_FORCES, TASK_FORCES)? {
                    reader.unit(unit?, ctask_force::VTABLE_RVA, &mut output, &mut counter)?;
                }
            }
            for unit in reader.members(country.pointer, ccountry::RAILWAY_UNITS, RAILWAY_GUNS)? {
                reader.unit(unit?, crailway_gun::VTABLE_RVA, &mut output, &mut counter)?;
            }
        }
        if divisions.is_some_and(|sources| next_division != sources.entries.len()) {
            return Err("shared division census references an absent country");
        }
        Ok(counter.outcome())
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Clone, Copy)]
struct Roster {
    header: &'static str,
    member: &'static str,
}

const DIVISIONS: Roster = Roster {
    header: "division roster unreadable or invalid",
    member: "division roster pointer unreadable or null",
};
const FLEETS: Roster = Roster {
    header: "fleet roster unreadable or invalid",
    member: "fleet roster pointer unreadable or null",
};
const TASK_FORCES: Roster = Roster {
    header: "task force roster unreadable or invalid",
    member: "task force roster pointer unreadable or null",
};
const RAILWAY_GUNS: Roster = Roster {
    header: "railway gun roster unreadable or invalid",
    member: "railway gun roster pointer unreadable or null",
};

struct Counter {
    len: usize,
    unread_rows: u32,
}

impl Counter {
    fn outcome(&self) -> Outcome {
        if self.unread_rows == 0 {
            Outcome::Rows { len: self.len }
        } else {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread_rows,
            }
        }
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}

impl<M: Memory> Reader<'_, M> {
    fn vtable(&self, pointer: usize, rva: Rva, reason: &'static str) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    fn members(
        &self,
        owner: usize,
        offset: usize,
        reasons: Roster,
    ) -> Result<impl Iterator<Item = Result<usize, &'static str>> + use<'_, M>, &'static str> {
        let array = owner
            .checked_add(offset)
            .and_then(|at| Array::read(self.memory, at, 8))
            .ok_or(reasons.header)?;
        Ok((0..array.count).map(move |index| {
            array
                .element(index, 8)
                .and_then(|at| read::<u64>(self.memory, at))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or(reasons.member)
        }))
    }

    fn divisions(
        &self,
        sources: &[DivisionSource],
        output: &mut Buffer<'_, Row>,
        counter: &mut Counter,
    ) -> Result<(), &'static str> {
        use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
        const BATCH: usize = 64;
        const FIELDS: [RecordField; 3] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: cunit::HANDLE,
                destination_offset: 8,
                len: 8,
            },
            RecordField {
                source_offset: cunit::VIEWER_DATA,
                destination_offset: 16,
                len: 16,
            },
        ];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 32).expect("valid record layout");
        let mut pointers = [0; BATCH];
        let mut bytes = [0; BATCH * 32];
        let mut blocks = [[0; ENTRY_BATCH * STRIDE]; GROUP];
        for chunk in sources.chunks(BATCH) {
            for (pointer, source) in pointers.iter_mut().zip(chunk) {
                *pointer = source.pointer;
            }
            let completed = self.memory.copy_records(
                RecordSources::Pointers(&pointers[..chunk.len()]),
                &LAYOUT,
                &mut bytes,
            );
            let expected = engine_sites::at(self.base, carmy::VTABLE_RVA).map(|v| v as u64);
            for record in bytes[..(completed / 3).min(chunk.len()) * 32].chunks_exact(32) {
                if Some(u64::from_le_bytes(record[..8].try_into().unwrap())) != expected {
                    continue;
                }
                if let Some(viewers) =
                    Array::from_header(record[16..32].try_into().unwrap(), STRIDE)
                {
                    let len = (viewers.count as usize).min(ENTRY_BATCH) * STRIDE;
                    if len != 0 {
                        self.memory.prefetch(viewers.data, len);
                    }
                }
            }
            for (group, sources) in chunk.chunks(GROUP).enumerate() {
                let start = group * GROUP;
                let read = self.first_blocks(
                    &bytes[start * 32..(start + sources.len()) * 32],
                    completed.saturating_sub(start * 3),
                    &mut blocks,
                );
                for (index, source) in sources.iter().enumerate() {
                    if source.pointer == 0 {
                        return Err(DIVISIONS.member);
                    }
                    let first = (start + index) * 32;
                    self.unit_fields(
                        bytes[first..first + 8].try_into().unwrap(),
                        bytes[first + 8..first + 16].try_into().unwrap(),
                        bytes[first + 16..first + 32].try_into().unwrap(),
                        completed.saturating_sub((start + index) * 3).min(3),
                        carmy::VTABLE_RVA,
                        read[index].map(|len| &blocks[index][..len]),
                        output,
                        counter,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn first_blocks(
        &self,
        records: &[u8],
        completed: usize,
        blocks: &mut [[u8; ENTRY_BATCH * STRIDE]; GROUP],
    ) -> [Option<usize>; GROUP] {
        let expected = engine_sites::at(self.base, carmy::VTABLE_RVA).map(|v| v as u64);
        let mut addresses = [0; GROUP];
        let mut lens = [0; GROUP];
        for (index, record) in records.chunks_exact(32).enumerate() {
            if completed < (index + 1) * 3
                || Some(u64::from_le_bytes(record[..8].try_into().unwrap())) != expected
            {
                continue;
            }
            if let Some(viewers) = Array::from_header(record[16..32].try_into().unwrap(), STRIDE) {
                addresses[index] = viewers.data;
                lens[index] = (viewers.count as usize).min(ENTRY_BATCH) * STRIDE;
            }
        }
        let mut destinations = blocks.each_mut().map(|block| &mut block[..]);
        for (destination, &len) in destinations.iter_mut().zip(&lens) {
            *destination = &mut std::mem::take(destination)[..len];
        }
        let read = self.memory.copy_scattered(addresses, &mut destinations);
        if lens[read.min(GROUP)..].iter().any(|&len| len != 0) {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::UnitIntel);
        }
        std::array::from_fn(|index| (index < read && lens[index] != 0).then_some(lens[index]))
    }

    fn unit(
        &self,
        unit: usize,
        vtable: Rva,
        output: &mut Buffer<'_, Row>,
        counter: &mut Counter,
    ) -> Result<(), &'static str> {
        let mut primary = [0; 8];
        let mut handle = [0; 8];
        let mut header = [0; 16];
        let complete = self.memory.copy_scattered(
            [0, cunit::HANDLE, cunit::VIEWER_DATA]
                .map(|offset| unit.checked_add(offset).unwrap_or(0)),
            &mut [&mut primary, &mut handle, &mut header],
        );
        self.unit_fields(
            primary, handle, header, complete, vtable, None, output, counter,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn unit_fields(
        &self,
        primary: [u8; 8],
        handle: [u8; 8],
        header: [u8; 16],
        complete: usize,
        vtable: Rva,
        first_block: Option<&[u8]>,
        output: &mut Buffer<'_, Row>,
        counter: &mut Counter,
    ) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, vtable).ok_or("module address overflow")?;
        if complete == 0 || u64::from_le_bytes(primary) != expected as u64 {
            return Err("unit vtable mismatch or identity unreadable");
        }
        if complete != 3 {
            return Err("unit handle or viewer vector header unreadable");
        }
        let id = u64::from_le_bytes(handle);
        if id == 0 {
            return Err("nonnull unit has a null reference handle");
        }
        let viewers =
            Array::from_header(header, STRIDE).ok_or("unit viewer vector header invalid")?;
        output.ensure(
            counter
                .len
                .checked_add(viewers.count as usize)
                .ok_or("unit viewer demand overflow")?,
        )?;
        let mut bytes = [0; ENTRY_BATCH * STRIDE];
        for batch in 0..(viewers.count as usize).div_ceil(ENTRY_BATCH).max(1) {
            let first = batch * ENTRY_BATCH;
            let count = (viewers.count as usize - first).min(ENTRY_BATCH);
            if count == 0 {
                break;
            }
            match first_block.filter(|block| first == 0 && block.len() == count * STRIDE) {
                Some(block) => bytes[..block.len()].copy_from_slice(block),
                None => {
                    let address = viewers
                        .element(first as u32, STRIDE)
                        .ok_or("unit viewer entry address overflow")?;
                    if !read_slice(self.memory, address, &mut bytes[..count * STRIDE]) {
                        return Err("unit viewer entry block unreadable");
                    }
                }
            }
            for (index, entry) in bytes[..count * STRIDE].chunks_exact(STRIDE).enumerate() {
                if counter.len == output.len() {
                    counter.unread_rows = counter
                        .unread_rows
                        .checked_add(1)
                        .ok_or("unit viewer omitted count overflow")?;
                    continue;
                }
                output[counter.len] = Row {
                    unit: id,
                    ordinal: (first + index) as u32,
                    viewer: u32::from_le_bytes(
                        entry[VIEWER..VIEWER + 4].try_into().expect("four bytes"),
                    ),
                    level: entry[LEVEL] as i8,
                };
                counter.len += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
