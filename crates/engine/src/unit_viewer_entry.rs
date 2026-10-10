//! Per-viewer unit visibility-level entries. Upsert selects by viewer
//! country ordinal, resets accumulator +8 and stores the signed level
//! byte at +0x10. Updates advance the accumulator by a per-class rate,
//! decrement level after the threshold and swap-remove entries below one.
//! The stored value depends on tick history rather than the current hour
//! positions, so it is recorded instead of derived.

use crate::{
    Memory, carmy, ccountry, cfleet,
    cpdx_array::Array,
    crailway_gun, ctask_force, cunit,
    memory::read,
    read_slice,
    walk::{Buffer, Outcome},
};

/// Viewer-entry stride.
pub const STRIDE: usize = 24;
/// Country key, u32 at +0, compared with the resolved country ordinal.
pub const VIEWER: usize = 0x00;
/// Decay accumulator, i64 at +8, incremented by a per-class rate and reset by upsert.
pub const TIMER: usize = 0x08;
/// Signed level byte at +0x10, decremented during decay.
pub const LEVEL: usize = 0x10;

/// One viewer entry in stored array order. `ordinal` is part of the key:
/// expired entries are removed by copying the tail over them, and the
/// serializer writes the resulting complete array order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native CUnit `+0x18` handle: type in the low 32 bits, id in the high 32 bits.
    pub unit: u64,
    pub ordinal: u32,
    /// Stored viewer key: the country's ordinal in `CGameState+0x310`, not a link id.
    pub viewer: u32,
    /// Native signed level byte; nonpositive entries are recorded, not dropped.
    pub level: i8,
}

/// One pointer in the native division roster, including counted nulls. The
/// canonical country slot preserves native country/roster ordering independently
/// of the capacity and private fields of the Divisions projection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionSource {
    pub pointer: usize,
    pub canonical: u32,
}

/// A complete callback-owned pointer census; this consumer authenticates only
/// its own primary identity, handle and viewer vector. Unrelated division field
/// failures must never be placed in `outcome`.
#[derive(Debug, Clone, Copy)]
pub struct DivisionSources<'a> {
    pub entries: &'a [DivisionSource],
    pub outcome: Outcome,
}

/// Reads the per-viewer level array of every division, task force and railway
/// gun reachable from the canonical country rosters.
///
/// The roots are exactly the ones the Divisions, TaskForces and
/// RailwayGunCounterInputs tables use (each country's division, fleet task-force
/// and railway-gun rosters), so a unit produces at most one set of rows and its
/// handle joins those tables without a bridge. A null pointer inside a counted
/// roster, a failed vtable, a zero handle, an out-of-bounds vector header or an
/// unreadable entry block refuses the whole table: the engine's own walks
/// dereference these unconditionally, so a hole is a torn read rather than a
/// state. Capacity omits whole entries and reports the remaining demand. Nothing
/// here allocates, retries or calls the engine.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_with_divisions(memory, base, game_state, output, None)
}

/// Read viewer inputs from a shared native division census without repeating
/// any country division-vector header or pointer reads. Fleet and railway-gun
/// inputs retain their independent authentication and their native position
/// after divisions within each country.
pub fn walk_with_divisions(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    divisions: Option<DivisionSources<'_>>,
) -> Outcome {
    walk_with_divisions_buffer(memory, base, game_state, Buffer::Fixed(output), divisions)
}

/// Worker-owned storage grows from the original native headers.
pub fn walk_with_divisions_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    divisions: Option<DivisionSources<'_>>,
) -> Outcome {
    walk_with_divisions_buffer(memory, base, game_state, Buffer::Growing(output), divisions)
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

/// One roster's two refusal reasons, so a fault names the vector it came from.
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
    fn vtable(&self, pointer: usize, rva: usize, reason: &'static str) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    /// Every counted slot of one pointer vector; a null slot is a refusal.
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
        use crate::memory::{RecordField, RecordLayout, RecordSources};
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
        for chunk in sources.chunks(BATCH) {
            for (pointer, source) in pointers.iter_mut().zip(chunk) {
                *pointer = source.pointer;
            }
            let completed = self.memory.copy_records(
                RecordSources::Pointers(&pointers[..chunk.len()]),
                &LAYOUT,
                &mut bytes,
            );
            for (index, source) in chunk.iter().enumerate() {
                if source.pointer == 0 {
                    return Err(DIVISIONS.member);
                }
                let first = index * 32;
                self.unit_fields(
                    bytes[first..first + 8].try_into().unwrap(),
                    bytes[first + 8..first + 16].try_into().unwrap(),
                    bytes[first + 16..first + 32].try_into().unwrap(),
                    completed.saturating_sub(index * 3).min(3),
                    carmy::VTABLE_RVA,
                    output,
                    counter,
                )?;
            }
        }
        Ok(())
    }

    fn unit(
        &self,
        unit: usize,
        vtable: usize,
        output: &mut Buffer<'_, Row>,
        counter: &mut Counter,
    ) -> Result<(), &'static str> {
        let mut primary = [0; 8];
        let mut handle = [0; 8];
        let mut header = [0; 16];
        // One guarded gather of the vtable, the reference handle and the vector
        // header: no intervening CUnit bytes are copied.
        let complete = self.memory.copy_scattered(
            [0, cunit::HANDLE, cunit::VIEWER_DATA]
                .map(|offset| unit.checked_add(offset).unwrap_or(0)),
            &mut [&mut primary, &mut handle, &mut header],
        );
        self.unit_fields(primary, handle, header, complete, vtable, output, counter)
    }

    #[allow(clippy::too_many_arguments)]
    fn unit_fields(
        &self,
        primary: [u8; 8],
        handle: [u8; 8],
        header: [u8; 16],
        complete: usize,
        vtable: usize,
        output: &mut Buffer<'_, Row>,
        counter: &mut Counter,
    ) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(vtable)
            .ok_or("module address overflow")?;
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
        // Unit-intel queries consume every stored viewer entry.
        output.ensure(
            counter
                .len
                .checked_add(viewers.count as usize)
                .ok_or("unit viewer demand overflow")?,
        )?;
        const ENTRIES: usize = 32;
        let mut bytes = [0; ENTRIES * STRIDE];
        for batch in 0..(viewers.count as usize).div_ceil(ENTRIES).max(1) {
            let first = batch * ENTRIES;
            let count = (viewers.count as usize - first).min(ENTRIES);
            if count == 0 {
                break;
            }
            let address = viewers
                .element(first as u32, STRIDE)
                .ok_or("unit viewer entry address overflow")?;
            if !read_slice(self.memory, address, &mut bytes[..count * STRIDE]) {
                return Err("unit viewer entry block unreadable");
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
