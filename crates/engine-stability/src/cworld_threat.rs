//! CWorldThreat: the world-tension manager, its ordered contribution vector and
//! its per-country aggregate cache.
//!
//! Rebuilding sizes and clears the aggregate array from the live country census,
//! then sums every current contribution into the total and its canonical-country
//! slot. Exhausted sources are removed before the total is rebuilt, clamped and
//! divided for storage as the world-tension scalar. The recorded source sum and
//! resident scalar are independent operands; this reader neither scales nor clamps them.

use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    cthreat_source::{self, Row as SourceRow},
    read,
    walk::{Buffer, Outcome},
};

/// CWorldThreat primary vtable.
pub const VTABLE_RVA: usize = 0x029f_2f08;
/// Stored signed i64 total, after clamping and scalar conversion.
pub const TOTAL: usize = 0x08;
/// CWorldThreat CThreatSource pointer array.
pub const SOURCES: usize = 0x10;
/// CWorldThreat signed i64 country aggregate array.
pub const COUNTRY_AGGREGATES: usize = 0x28;
/// CWorldThreat aggregate element stride.
pub const AGGREGATE_STRIDE: usize = 0x08;
/// CWorldThreat source-pointer stride.
pub const SOURCE_STRIDE: usize = 0x08;
/// CWorldThreat total update.
pub const UPDATE_TOTAL_RVA: usize = 0x00f8_3a60;
/// World-tension clamp ceiling.
pub const CLAMP_CEILING: i64 = 10_000_000;
/// World-tension scalar divisor.
pub const SCALAR_DIVISOR: i64 = 100;

/// The manager's own stored words. Counts are the engine's, not the walker's.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManagerRow {
    /// Resident world-tension scalar after total update.
    pub total_raw: i64,
    pub source_count: i32,
    pub country_aggregate_count: i32,
}

/// One entry of the rebuilt per-country cache, keyed by canonical country slot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AggregateRow {
    pub country_slot: u32,
    /// Signed sum of every source contribution attributed to this slot.
    pub total_raw: i64,
}

/// One contribution walk: its outcome and the arena bytes it used and needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceOutcome {
    pub outcome: Outcome,
    pub bytes_len: usize,
    /// Full reason-byte need of the rows this pass actually reached.
    pub required_bytes: usize,
}

impl SourceOutcome {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

/// Authenticated manager header words and array descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reader {
    row: ManagerRow,
    sources: Array,
    aggregates: Array,
}

impl Reader {
    /// Authenticate `cgame_state::WORLD_THREAT` and read both array headers once.
    pub fn new(memory: &impl Memory, base: usize, game_state: usize) -> Result<Self, &'static str> {
        let pointer = game_state
            .checked_add(cgame_state::WORLD_THREAT)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("world threat pointer unreadable or null")?;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("world threat module address overflow")?;
        let head = read::<[u8; 0x40]>(memory, pointer).ok_or("world threat header unreadable")?;
        if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
            return Err("world threat vtable mismatch");
        }
        let header = |offset: usize| -> [u8; 16] { head[offset..offset + 16].try_into().unwrap() };
        let sources = Array::from_header(header(SOURCES), SOURCE_STRIDE)
            .ok_or("world threat source array invalid")?;
        let aggregates = Array::from_header(header(COUNTRY_AGGREGATES), AGGREGATE_STRIDE)
            .ok_or("world threat aggregate array invalid")?;
        Ok(Self {
            row: ManagerRow {
                total_raw: i64::from_le_bytes(head[TOTAL..TOTAL + 8].try_into().unwrap()),
                source_count: sources.count as i32,
                country_aggregate_count: aggregates.count as i32,
            },
            sources,
            aggregates,
        })
    }

    /// One manager row, or `Unavailable` when the caller supplied no slot.
    pub fn manager(&self, output: &mut [ManagerRow]) -> Outcome {
        let Some(slot) = output.first_mut() else {
            return Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            };
        };
        *slot = self.row;
        Outcome::Rows { len: 1 }
    }

    /// Authenticated native extents; sizing hints never supply recorded values.
    pub fn counts(&self) -> [usize; 2] {
        [self.sources.count as usize, self.aggregates.count as usize]
    }

    /// Serialization visits every stored contribution and preserves each complete reason string.
    pub fn reason_demand(&self, memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
        let mut bytes = 0_usize;
        for ordinal in 0..self.sources.count {
            let pointer = self
                .sources
                .element(ordinal, SOURCE_STRIDE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0)
                .ok_or("threat source pointer unreadable or null")?;
            let (_, _, wanted) =
                cthreat_source::read_row(memory, base, ordinal, pointer, &mut [], 0)?;
            bytes = bytes
                .checked_add(wanted)
                .ok_or("threat source reason demand overflow")?;
        }
        Ok(bytes)
    }

    /// Every contribution in stored order, with its literal reason bytes copied
    /// into `reasons`. A null slot inside the counted span is a fault, not a hole.
    /// A row whose reason does not fit the arena is omitted and counted, like a
    /// row that does not fit `output`; `required` always reports the full need.
    pub fn sources(
        &self,
        memory: &impl Memory,
        base: usize,
        output: &mut [SourceRow],
        reasons: &mut [u8],
    ) -> SourceOutcome {
        self.sources_buffer(memory, base, Buffer::Fixed(output), Buffer::Fixed(reasons))
    }

    pub fn sources_growing(
        &self,
        memory: &impl Memory,
        base: usize,
        output: &mut Vec<SourceRow>,
        reasons: &mut Vec<u8>,
    ) -> SourceOutcome {
        self.sources_buffer(
            memory,
            base,
            Buffer::Growing(output),
            Buffer::Growing(reasons),
        )
    }

    fn sources_buffer(
        &self,
        memory: &impl Memory,
        base: usize,
        mut output: Buffer<'_, SourceRow>,
        mut reasons: Buffer<'_, u8>,
    ) -> SourceOutcome {
        // Serialization retains every stored contribution occurrence.
        // Continue from this reader's original header, without another read.
        if let Err(reason) = output.ensure(self.sources.count as usize) {
            return SourceOutcome::unavailable(reason);
        }
        let mut len = 0;
        let mut used = 0;
        let mut required = 0_usize;
        let mut unread_rows = 0_u32;
        for ordinal in 0..self.sources.count {
            let pointer = self
                .sources
                .element(ordinal, SOURCE_STRIDE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0);
            let Some(pointer) = pointer else {
                return SourceOutcome::unavailable("threat source pointer unreadable or null");
            };
            if len == output.len() {
                // No further row can be published, and an unread source's reason
                // length is unknown without reading it, so stop counting bytes.
                // Rows already omitted for a full arena stay counted.
                unread_rows += self.sources.count - ordinal;
                break;
            }
            match cthreat_source::read_row_buffer(
                memory,
                base,
                ordinal,
                pointer,
                &mut reasons,
                used,
            ) {
                Ok((row, next, wanted)) => {
                    required = required.saturating_add(wanted);
                    match row {
                        Some(row) => {
                            output[len] = row;
                            len += 1;
                            used = next;
                        }
                        None => unread_rows += 1,
                    }
                }
                Err(reason) => return SourceOutcome::unavailable(reason),
            }
        }
        SourceOutcome {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len: used,
            required_bytes: required,
        }
    }

    /// The dense aggregate cache, including slot zero and zero-valued slots.
    pub fn aggregates(&self, memory: &impl Memory, output: &mut [AggregateRow]) -> Outcome {
        let len = output.len().min(self.aggregates.count as usize);
        for (slot, destination) in output[..len].iter_mut().enumerate() {
            let value = self
                .aggregates
                .element(slot as u32, AGGREGATE_STRIDE)
                .and_then(|address| read::<i64>(memory, address));
            let Some(total_raw) = value else {
                return Outcome::Unavailable {
                    reason: "threat country aggregate unreadable",
                };
            };
            *destination = AggregateRow {
                country_slot: slot as u32,
                total_raw,
            };
        }
        let unread_rows = self.aggregates.count - len as u32;
        if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{memory::SliceMemory, msvc_string};

    const BASE: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const SOURCE_ARRAY: usize = 0x3000;
    const AGGREGATES: usize = 0x3100;
    const FIRST: usize = 0x4000;
    const SECOND: usize = 0x4100;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn source(bytes: &mut [u8], at: usize, current: i64, reason: &[u8]) {
        put(
            bytes,
            at,
            &(cthreat_source::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(bytes, at + cthreat_source::COUNTRY, &7_u32.to_le_bytes());
        put(bytes, at + cthreat_source::CURRENT, &current.to_le_bytes());
        put(bytes, at + cthreat_source::FINAL, &current.to_le_bytes());
        put(bytes, at + cthreat_source::REASON, reason);
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::LENGTH,
            &(reason.len() as u64 - 1).to_le_bytes(),
        );
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x4000];
        put(
            &mut bytes,
            BASE + cgame_state::WORLD_THREAT,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(&mut bytes, MANAGER, &(VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, MANAGER + TOTAL, &(-125_682_i64).to_le_bytes());
        array(&mut bytes, MANAGER + SOURCES, SOURCE_ARRAY, 2);
        array(&mut bytes, MANAGER + COUNTRY_AGGREGATES, AGGREGATES, 3);
        put(&mut bytes, SOURCE_ARRAY, &(FIRST as u64).to_le_bytes());
        put(&mut bytes, SOURCE_ARRAY + 8, &(SECOND as u64).to_le_bytes());
        put(&mut bytes, AGGREGATES, &0_i64.to_le_bytes());
        put(&mut bytes, AGGREGATES + 8, &i64::MIN.to_le_bytes());
        put(&mut bytes, AGGREGATES + 16, &i64::MAX.to_le_bytes());
        source(&mut bytes, FIRST, -125_682, b"Peace\0");
        source(&mut bytes, SECOND, i64::MAX, b"\0");
        bytes
    }

    #[test]
    fn manager_total_ordered_sources_and_dense_aggregates_keep_native_words() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, BASE).unwrap();
        let mut manager = [ManagerRow::default(); 1];
        assert_eq!(reader.manager(&mut manager), Outcome::Rows { len: 1 });
        assert_eq!(
            manager[0],
            ManagerRow {
                total_raw: -125_682,
                source_count: 2,
                country_aggregate_count: 3,
            }
        );
        assert_eq!(
            reader.manager(&mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        let mut sources = [SourceRow::default(); 4];
        let mut reasons = [0; 32];
        let result = reader.sources(&memory, 0, &mut sources, &mut reasons);
        assert_eq!(
            result,
            SourceOutcome {
                outcome: Outcome::Rows { len: 2 },
                bytes_len: 5,
                required_bytes: 5,
            }
        );
        assert_eq!(
            (
                sources[0].ordinal,
                sources[0].current_raw,
                sources[0].country
            ),
            (0, -125_682, 7)
        );
        assert_eq!(&reasons[..5], b"Peace");
        assert_eq!(
            (
                sources[1].ordinal,
                sources[1].reason_len,
                sources[1].final_raw
            ),
            (1, 0, i64::MAX)
        );
        let mut aggregates = [AggregateRow::default(); 4];
        assert_eq!(
            reader.aggregates(&memory, &mut aggregates),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            &aggregates[..3],
            &[
                AggregateRow {
                    country_slot: 0,
                    total_raw: 0
                },
                AggregateRow {
                    country_slot: 1,
                    total_raw: i64::MIN
                },
                AggregateRow {
                    country_slot: 2,
                    total_raw: i64::MAX
                },
            ]
        );
    }

    #[test]
    fn capacity_and_null_slots_are_reported_not_hidden() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, BASE).unwrap();
        let mut sources = [SourceRow::default(); 1];
        assert_eq!(
            reader
                .sources(&memory, 0, &mut sources, &mut [0; 32])
                .outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            reader.aggregates(&memory, &mut [AggregateRow::default(); 1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        // An arena too small for a reason omits that row and still reports it.
        assert_eq!(
            reader.sources(&memory, 0, &mut [SourceRow::default(); 4], &mut []),
            SourceOutcome {
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                },
                bytes_len: 0,
                required_bytes: 5,
            }
        );
        // Arena omissions and a full row buffer add up instead of replacing each
        // other: source 0 needs five arena bytes it cannot have, source 1 has an
        // empty reason and is published, and the third source finds no room.
        let mut three = bytes.clone();
        put(&mut three, MANAGER + SOURCES + 12, &3_u32.to_le_bytes());
        put(&mut three, MANAGER + SOURCES + 8, &3_u32.to_le_bytes());
        put(
            &mut three,
            SOURCE_ARRAY + 16,
            &(SECOND as u64).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: BASE,
            bytes: &three,
        };
        let reader = Reader::new(&memory, 0, BASE).unwrap();
        assert_eq!(
            reader.sources(&memory, 0, &mut [SourceRow::default(); 1], &mut []),
            SourceOutcome {
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 2
                },
                bytes_len: 0,
                required_bytes: 5,
            }
        );

        let mut broken = bytes.clone();
        put(&mut broken, SOURCE_ARRAY + 8, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &broken,
        };
        let reader = Reader::new(&memory, 0, BASE).unwrap();
        assert_eq!(
            reader
                .sources(&memory, 0, &mut [SourceRow::default(); 4], &mut [0; 32])
                .outcome,
            Outcome::Unavailable {
                reason: "threat source pointer unreadable or null"
            }
        );
    }

    #[test]
    fn broken_roots_and_headers_never_become_an_empty_manager() {
        for (address, value) in [
            (BASE + cgame_state::WORLD_THREAT, 0_u64),
            (MANAGER, 0),
            (MANAGER + SOURCES, 0),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &value.to_le_bytes());
            if address == MANAGER + SOURCES {
                // A null pointer with a nonzero count is a corrupt header.
                put(&mut bytes, MANAGER + SOURCES + 12, &2_u32.to_le_bytes());
            }
            let outcome = Reader::new(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                0,
                BASE,
            );
            assert!(outcome.is_err(), "{address:#x}");
        }
        assert!(
            Reader::new(
                &SliceMemory {
                    base: BASE,
                    bytes: &fixture()[..MANAGER - BASE + 0x20]
                },
                0,
                BASE,
            )
            .is_err()
        );
    }
}
