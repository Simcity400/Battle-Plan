use super::*;
use engine_allocation::{owned_vec, reserve_map, vec};

#[derive(Debug, Clone)]
pub(super) struct TraitDefinition {
    address: usize,
    identity: usize,
    ordinal: usize,
    literal: usize,
    source: usize,
    len: usize,
    inline: bool,
    token: usize,
    terminated: bool,
    error: Option<&'static str>,
}

#[derive(Debug, Default)]
pub(super) struct TraitDefinitions {
    pub(super) entries: Vec<TraitDefinition>,
    by_address: std::collections::HashMap<usize, usize>,
    log: collector_raw::raw::RawLog<1>,
    charge: collector_primitives::raw_memory::Charge,
}

impl TraitDefinitions {
    pub(super) fn new() -> Self {
        let hint = demand::initial_traits();
        let (rows, bytes) = (hint.rows, hint.bytes);
        let mut value = Self {
            entries: owned_vec(vec(rows)),
            by_address: std::collections::HashMap::default(),
            log: collector_raw::raw::RawLog::new(
                rows.saturating_mul(8),
                rows.saturating_mul(64).saturating_add(bytes),
            ),
            ..Self::default()
        };
        let _ = reserve_map(&mut value.by_address, rows);
        value.update_charge();
        value
    }

    pub(super) fn update_charge(&mut self) {
        use collector_primitives::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: vector_bytes(&self.entries),
            index_map_entries_lower_bound: (self.by_address.capacity()
                * std::mem::size_of::<(usize, usize)>())
                as u64,
            ..Capacity::default()
        });
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.by_address.clear();
        self.log.clear();
    }

    pub(super) fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub(super) fn overflowed(&self) -> bool {
        self.log.overflowed
    }

    pub(super) fn grow(&mut self) {
        self.log.grow();
    }

    fn room(&mut self) -> Result<(), &'static str> {
        engine_allocation::reserve(&mut self.entries, 1)
            .map_err(|_| "character trait definition allocation failed")
    }

    pub(super) fn header(
        &mut self,
        memory: &impl Memory,
        base: usize,
        address: usize,
    ) -> Result<usize, &'static str> {
        if let Some(&index) = self.by_address.get(&address) {
            return Ok(index);
        }
        self.room()?;
        let mut definition = TraitDefinition {
            address,
            identity: 0,
            ordinal: 0,
            literal: 0,
            source: 0,
            len: 0,
            inline: false,
            token: 0,
            terminated: false,
            error: None,
        };
        definition.error = (|| {
            let at = |offset| {
                address
                    .checked_add(offset)
                    .ok_or("unit leader address overflow")
            };
            let expected = engine_sites::at(base, cunit_leader_trait::VTABLE_RVA)
                .ok_or("unit leader address overflow")?;
            let (starts, completed) = self
                .log
                .take_scattered_prefetched(
                    memory,
                    [
                        address,
                        at(cunit_leader_trait::ORDINAL)?,
                        at(cunit_leader_trait::TOKEN)?,
                    ],
                    [12, 4, 32],
                    [1; 3],
                )
                .ok_or("character trait definition staging capacity exhausted")?;
            definition.identity = starts[0];
            definition.ordinal = starts[1];
            definition.literal = starts[2];
            if completed == 0 {
                return Err("unit leader trait identity unreadable");
            }
            if u64::from_le_bytes(self.log.bytes[starts[0]..starts[0] + 8].try_into().unwrap())
                != expected as u64
            {
                return Err("unit leader trait vtable mismatch");
            }
            if completed != 3 {
                return Err("unit leader trait identity unreadable");
            }
            let header = &self.log.bytes[starts[2]..starts[2] + 32];
            let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
            let capacity = u64::from_le_bytes(header[24..].try_into().unwrap());
            if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
                return Err("unit leader string header invalid");
            }
            definition.len =
                usize::try_from(len).map_err(|_| "unit leader string length overflow")?;
            definition.inline = capacity == msvc_string::INLINE_CAPACITY;
            definition.source = if definition.inline {
                at(cunit_leader_trait::TOKEN)?
            } else {
                usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("unit leader string pointer null or invalid")?
            };
            Ok(())
        })()
        .err();
        let index = self.entries.len();
        self.entries.push(definition);
        Ok(index)
    }

    pub(super) fn result(&self, index: usize) -> Result<(), &'static str> {
        self.entries[index].error.map_or(Ok(()), Err)
    }

    pub(super) fn token_len(&self, index: usize) -> usize {
        self.entries[index].len
    }

    pub(super) fn consume(
        &mut self,
        memory: &impl Memory,
        index: usize,
    ) -> Result<usize, &'static str> {
        if self.entries[index].error.is_some() || self.entries[index].terminated {
            return Ok(index);
        }
        let definition = &mut self.entries[index];
        definition.error = (|| {
            if definition.inline {
                if self.log.bytes[definition.literal + definition.len] != 0 {
                    return Err("unit leader string terminator invalid or unreadable");
                }
                definition.token = definition.literal;
            } else {
                let terminator = definition
                    .source
                    .checked_add(definition.len)
                    .ok_or("unit leader address overflow")?;
                let (starts, completed) = self
                    .log
                    .take_scattered(
                        memory,
                        [terminator, definition.source],
                        [1, definition.len],
                        [1; 2],
                    )
                    .ok_or("character trait definition staging capacity exhausted")?;
                if completed == 0 || self.log.bytes[starts[0]] != 0 {
                    return Err("unit leader string terminator invalid or unreadable");
                }
                if completed != 2 {
                    return Err("unit leader trait token bytes unreadable");
                }
                definition.token = starts[1];
            }
            Ok(())
        })()
        .err();
        if definition.error.is_none() {
            definition.terminated = true;
            if !self.by_address.contains_key(&definition.address)
                && self.by_address.len() == self.by_address.capacity()
            {
                reserve_map(&mut self.by_address, 1)
                    .map_err(|_| "character trait definition index allocation failed")?;
            }
            self.by_address.insert(definition.address, index);
        }
        Ok(index)
    }

    pub(super) fn project(&self, index: usize) -> (i32, i32, &[u8]) {
        let definition = &self.entries[index];
        (
            i32::from_le_bytes(
                self.log.bytes[definition.identity + 8..definition.identity + 12]
                    .try_into()
                    .unwrap(),
            ),
            i32::from_le_bytes(
                self.log.bytes[definition.ordinal..definition.ordinal + 4]
                    .try_into()
                    .unwrap(),
            ),
            &self.log.bytes[definition.token..definition.token + definition.len],
        )
    }
}

#[derive(Debug)]
pub(super) struct TraitOccurrence {
    ordinal: u32,
    definition: usize,
}

#[derive(Debug)]
pub(super) struct TraitVector {
    pub(super) unit: usize,
    pub(super) leader: u32,
    start: usize,
    len: usize,
    pub(super) outcome: Outcome,
    pub(super) bytes_len: usize,
    required_bytes: usize,
}

#[derive(Debug, Default)]
pub(super) struct TraitCapture {
    pub(super) definitions: TraitDefinitions,
    pub(super) vectors: Vec<TraitVector>,
    occurrences: Vec<TraitOccurrence>,
    pub(super) fault: bool,
    charge: collector_primitives::raw_memory::Charge,
}

impl TraitCapture {
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn with_test_capacities(
        vectors: usize,
        occurrences: usize,
        definitions: usize,
        bytes: usize,
    ) -> Self {
        let mut value = Self {
            definitions: TraitDefinitions {
                entries: owned_vec(vec(definitions)),
                log: collector_raw::raw::RawLog::new(definitions.saturating_mul(8), bytes),
                ..TraitDefinitions::default()
            },
            vectors: owned_vec(vec(vectors)),
            occurrences: owned_vec(vec(occurrences)),
            ..Self::default()
        };
        let _ = reserve_map(&mut value.definitions.by_address, definitions);
        value.definitions.update_charge();
        value.update_charge();
        value
    }

    pub(super) fn new() -> Self {
        let hint = demand::initial_traits();
        let mut value = Self {
            definitions: TraitDefinitions::new(),
            vectors: owned_vec(vec(census::initial_len())),
            occurrences: owned_vec(vec(hint.rows)),
            ..Self::default()
        };
        value.update_charge();
        value
    }

    pub(super) fn update_charge(&mut self) {
        use collector_primitives::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: vector_bytes(&self.vectors) + vector_bytes(&self.occurrences),
            ..Capacity::default()
        });
    }

    pub(super) fn clear(&mut self) {
        self.definitions.clear();
        self.vectors.clear();
        self.occurrences.clear();
        self.fault = false;
    }

    pub(super) fn grow_for_next(&self) -> bool {
        self.definitions.grow_for_next()
    }

    pub(super) fn overflowed(&self) -> bool {
        self.definitions.overflowed()
    }

    pub(super) fn grow(&mut self) {
        self.definitions.grow();
    }

    pub(super) fn begin(&mut self) {
        self.clear();
    }

    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        params: TraitVectorCopy,
    ) -> Result<(), &'static str> {
        engine_allocation::reserve(&mut self.vectors, 1)
            .map_err(|_| "character trait vector allocation failed")?;
        let mut vector = TraitVector {
            unit: params.unit,
            leader: params.leader_id,
            start: self.occurrences.len(),
            len: 0,
            outcome: Outcome::Rows { len: 0 },
            bytes_len: 0,
            required_bytes: 0,
        };
        let result = (|| -> Result<(), &'static str> {
            let identity = read::<[u8; 16]>(memory, params.unit).ok_or_else(|| {
                self.fault = true;
                "unit leader identity unreadable"
            })?;
            let found = usize::try_from(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                .ok()
                .and_then(|vtable| {
                    engine_sites::which(
                        base,
                        vtable,
                        &[
                            cunit_leader::VTABLE_RVA,
                            carmy_leader::VTABLE_RVA,
                            cnavy_leader::VTABLE_RVA,
                            engine_native_parts::coperative_leader_layout::VTABLE_RVA,
                        ],
                    )
                });
            if found.is_none() {
                return Err("unit leader vtable mismatch");
            }
            if u32::from_le_bytes(identity[8..12].try_into().unwrap())
                != cunit_leader::REFERENCE_TYPE
            {
                return Err("unit leader reference type mismatch");
            }
            if u32::from_le_bytes(identity[12..].try_into().unwrap()) != params.leader_id {
                return Err("unit leader trait parent identity mismatch");
            }
            let at = params
                .unit
                .checked_add(cunit_leader::TRAIT_DATA)
                .ok_or("unit leader address overflow")?;
            let header = read::<[u8; 16]>(memory, at);
            if header.is_none() {
                self.fault = true;
            }
            let array = header
                .and_then(|header| Array::from_header(header, TRAIT_STRIDE))
                .ok_or("unit leader trait vector invalid or unreadable")?;
            engine_allocation::reserve(&mut self.occurrences, array.count as usize)
                .map_err(|_| "character trait occurrence allocation failed")?;
            engine_sites::at(base, cunit_leader_trait::VTABLE_RVA)
                .ok_or("unit leader address overflow")?;
            for ordinal in 0..array.count {
                let address = array
                    .element(ordinal, TRAIT_STRIDE)
                    .ok_or("unit leader trait entry address overflow")?;
                let mut entry = [0u8; TRAIT_STRIDE];
                if !memory.copy(address, &mut entry) {
                    self.fault = true;
                    return Err("unit leader trait pointer unreadable");
                }
                let address = usize::try_from(u64::from_le_bytes(entry))
                    .map_err(|_| "unit leader trait pointer overflow")?;
                let definition = self.definitions.header(memory, base, address)?;
                self.definitions.result(definition)?;
                let len = self.definitions.token_len(definition);
                vector.required_bytes = vector
                    .required_bytes
                    .checked_add(len)
                    .ok_or("unit leader trait byte count overflow")?;
                let definition = self.definitions.consume(memory, definition)?;
                self.definitions.result(definition)?;
                self.occurrences.push(TraitOccurrence {
                    ordinal,
                    definition,
                });
                vector.len += 1;
                vector.bytes_len += len;
            }
            vector.outcome = Outcome::Rows { len: vector.len };
            Ok(())
        })();
        if let Err(reason) = result {
            vector.outcome = Outcome::Unavailable { reason };
        }
        self.vectors.push(vector);
        result
    }

    pub(super) fn finish(&mut self) {
        self.fault |= self.definitions.log.fault[0];
        self.update_charge();
        self.definitions.update_charge();
    }

    pub(super) fn demand(&self) -> Result<(usize, usize), &'static str> {
        self.vectors
            .iter()
            .try_fold(0_usize, |bytes, vector| {
                bytes
                    .checked_add(vector.bytes_len)
                    .ok_or("unit leader trait projection byte count overflow")
            })
            .map(|bytes| (self.occurrences.len(), bytes))
    }

    pub(super) fn project(
        &self,
        cursor: &mut usize,
        unit: usize,
        leader: u32,
        mut rows: engine_native_memory::walk::Buffer<'_, cunit_leader::TraitRow>,
        mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
    ) -> ReadResult {
        let Some(vector) = self.vectors.get(*cursor) else {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned vector missing",
            });
        };
        *cursor += 1;
        if vector.unit != unit || vector.leader != leader {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned parent mismatch",
            });
        }
        match rows
            .ensure(vector.len)
            .and_then(|fits| Ok(fits && bytes.ensure(vector.bytes_len)?))
        {
            Ok(true) => {}
            Ok(false) => {
                return ReadResult::rows(Outcome::Unavailable {
                    reason: "character trait owned projection capacity mismatch",
                });
            }
            Err(reason) => return ReadResult::rows(Outcome::Unavailable { reason }),
        }
        let mut start = 0;
        for (output, occurrence) in rows
            .iter_mut()
            .zip(&self.occurrences[vector.start..vector.start + vector.len])
        {
            let (token_id, definition_ordinal, token) =
                self.definitions.project(occurrence.definition);
            bytes[start..start + token.len()].copy_from_slice(token);
            *output = cunit_leader::TraitRow {
                leader_id: leader,
                ordinal: occurrence.ordinal,
                token_id,
                definition_ordinal,
                token_start: start,
                token_len: token.len(),
            };
            start += token.len();
        }
        if let Outcome::Unavailable { .. } = vector.outcome {
            return ReadResult::rows(vector.outcome);
        }
        ReadResult {
            outcome: vector.outcome,
            bytes_len: vector.bytes_len,
            required_bytes: vector.required_bytes,
        }
    }
}

pub(super) const TRAIT_STRIDE: usize = 8;

#[derive(Debug, Clone, Copy)]
pub(super) struct TraitVectorCopy {
    pub(super) unit: usize,
    pub(super) leader_id: u32,
}
