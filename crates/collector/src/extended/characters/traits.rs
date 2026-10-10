use super::*;
use engine::allocation::{owned_vec, reserve_map, vec};

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
    copied: bool,
    error: Option<&'static str>,
}

/// Each entry owns exact native observations. Only successful definitions enter
/// the address index, which is emptied before every pass. The owned log
/// supplies every definition byte consumed by the decode worker.
#[derive(Debug, Default)]
pub(super) struct TraitDefinitions {
    growing: bool,
    pub(super) entries: Vec<TraitDefinition>,
    by_address: std::collections::HashMap<usize, usize>,
    log: crate::raw::RawLog<2>,
    overflowed: bool,
    charge: crate::raw_memory::Charge,
}

impl TraitDefinitions {
    pub(super) fn new() -> Self {
        let hint = demand::initial_traits();
        let rows = hint[0].rows.saturating_add(hint[1].rows);
        let bytes = hint[0].bytes.saturating_add(hint[1].bytes);
        let mut value = Self {
            entries: owned_vec(vec(rows)),
            by_address: std::collections::HashMap::default(),
            log: crate::raw::RawLog::new(
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
        use crate::raw_memory::{Capacity, vector_bytes};
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
        self.overflowed = false;
    }

    pub(super) fn grow_for_next(&self) -> bool {
        self.overflowed || self.log.grow_for_next()
    }

    pub(super) fn overflowed(&self) -> bool {
        self.overflowed || self.log.overflowed
    }

    pub(super) fn grow(&mut self) {
        if self.overflowed {
            let required = self.entries.len().saturating_add(1);
            if engine::allocation::reserve_exact(&mut self.entries, 1).is_err() {
                return;
            }
            let additional = required.saturating_sub(self.by_address.len());
            if reserve_map(&mut self.by_address, additional).is_err() {
                return;
            }
            self.update_charge();
        }
        self.log.grow();
    }

    fn room(&mut self) -> Result<(), &'static str> {
        if self.growing {
            engine::allocation::reserve(&mut self.entries, 1)
                .map_err(|_| "character trait definition allocation failed")?;
        }
        if self.entries.len() == self.entries.capacity() {
            self.overflowed = true;
            Err("character trait definition staging capacity exhausted")
        } else {
            Ok(())
        }
    }

    pub(super) fn header(
        &mut self,
        memory: &impl Memory,
        base: usize,
        address: usize,
        table: usize,
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
            copied: false,
            error: None,
        };
        definition.error = (|| {
            let at = |offset| {
                address
                    .checked_add(offset)
                    .ok_or("unit leader address overflow")
            };
            let expected = base
                .checked_add(cunit_leader_trait::VTABLE_RVA)
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
                    [1 << table; 3],
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

    /// A byte-limited consumer witnesses only the terminator. Upgrading that
    /// observation creates a separate entry, so a payload fault cannot alter an
    /// earlier successful truncated observation in the other table.
    pub(super) fn consume(
        &mut self,
        memory: &impl Memory,
        mut index: usize,
        table: usize,
        fits: bool,
    ) -> Result<usize, &'static str> {
        if self.entries[index].error.is_some() {
            return Ok(index);
        }
        if self.entries[index].terminated && (!fits || self.entries[index].copied) {
            return Ok(index);
        }
        if self.entries[index].terminated {
            self.room()?;
            let upgraded = self.entries[index].clone();
            index = self.entries.len();
            self.entries.push(upgraded);
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
                if fits {
                    let (starts, completed) = self
                        .log
                        .take_scattered(
                            memory,
                            [terminator, definition.source],
                            [1, definition.len],
                            [1 << table; 2],
                        )
                        .ok_or("character trait definition staging capacity exhausted")?;
                    if completed == 0 || self.log.bytes[starts[0]] != 0 {
                        return Err("unit leader string terminator invalid or unreadable");
                    }
                    if completed != 2 {
                        return Err("unit leader trait token bytes unreadable");
                    }
                    definition.token = starts[1];
                } else {
                    let terminated = self
                        .log
                        .take(memory, terminator, 1, 1 << table)
                        .is_some_and(|start| self.log.bytes[start] == 0);
                    if !terminated {
                        return Err("unit leader string terminator invalid or unreadable");
                    }
                }
            }
            Ok(())
        })()
        .err();
        if definition.error.is_none() {
            definition.terminated = true;
            definition.copied = fits;
            if !self.by_address.contains_key(&definition.address)
                && self.by_address.len() == self.by_address.capacity()
            {
                if self.growing {
                    reserve_map(&mut self.by_address, 1)
                        .map_err(|_| "character trait definition index allocation failed")?;
                } else {
                    self.overflowed = true;
                    return Err("character trait definition index capacity exhausted");
                }
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
    entry: [u8; 16],
    ordinal: u32,
    definition: usize,
}

#[derive(Debug)]
pub(super) struct TraitVector {
    pub(super) unit: usize,
    leader: u32,
    start: usize,
    len: usize,
    pub(super) outcome: Outcome,
    bytes_len: usize,
    required_bytes: usize,
}

/// Current-pass occurrences retain native order, while successful definitions
/// are shared only inside this owned image.
#[derive(Debug, Default)]
pub(super) struct TraitCapture {
    pub(super) growing: bool,
    pub(super) definitions: TraitDefinitions,
    pub(super) vectors: [Vec<TraitVector>; 2],
    occurrences: [Vec<TraitOccurrence>; 2],
    rows: [usize; 2],
    bytes: [usize; 2],
    used_rows: [usize; 2],
    used_bytes: [usize; 2],
    truncated: [bool; 2],
    pub(super) fault: [bool; 2],
    pub(super) raw_bytes: [u64; 2],
    overflowed: bool,
    charge: crate::raw_memory::Charge,
}

impl TraitCapture {
    #[cfg(test)]
    pub(super) fn with_test_capacities(
        vectors: usize,
        occurrences: [usize; 2],
        definitions: usize,
        bytes: usize,
    ) -> Self {
        let mut value = Self {
            definitions: TraitDefinitions {
                entries: owned_vec(vec(definitions)),
                log: crate::raw::RawLog::new(definitions.saturating_mul(8), bytes),
                ..TraitDefinitions::default()
            },
            vectors: std::array::from_fn(|_| owned_vec(vec(vectors))),
            occurrences: std::array::from_fn(|index| owned_vec(vec(occurrences[index]))),
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
            vectors: std::array::from_fn(|_| owned_vec(vec(census::initial_len()))),
            occurrences: std::array::from_fn(|index| owned_vec(vec(hint[index].rows))),
            ..Self::default()
        };
        value.update_charge();
        value
    }

    pub(super) fn update_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: self.vectors.iter().map(vector_bytes).sum::<u64>()
                + self.occurrences.iter().map(vector_bytes).sum::<u64>(),
            ..Capacity::default()
        });
    }

    pub(super) fn clear(&mut self) {
        self.definitions.clear();
        for vector in &mut self.vectors {
            vector.clear();
        }
        for occurrences in &mut self.occurrences {
            occurrences.clear();
        }
        self.used_rows = [0; 2];
        self.used_bytes = [0; 2];
        self.truncated = [false; 2];
        self.fault = [false; 2];
        self.raw_bytes = [0; 2];
        self.overflowed = false;
    }

    pub(super) fn grow_for_next(&self) -> bool {
        self.overflowed || self.definitions.grow_for_next()
    }

    pub(super) fn overflowed(&self) -> bool {
        self.overflowed || self.definitions.overflowed()
    }

    pub(super) fn grow(&mut self) {
        if self.overflowed {
            for vector in &mut self.vectors {
                if vector.len() == vector.capacity() {
                    let _ = engine::allocation::reserve(vector, 1);
                }
            }
            for occurrences in &mut self.occurrences {
                if occurrences.len() == occurrences.capacity() {
                    let _ = engine::allocation::reserve(occurrences, 1);
                }
            }
            self.update_charge();
        }
        self.definitions.grow();
    }

    pub(super) fn begin(&mut self, context: CharactersCopyContext, growing: bool) {
        self.clear();
        self.growing = growing;
        self.definitions.growing = growing;
        if growing {
            self.definitions.log.enable_worker_growth();
        }
        self.rows = context.trait_rows;
        self.bytes = context.trait_bytes;
    }

    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        params: TraitVectorCopy,
    ) -> Result<(), &'static str> {
        let table = usize::from(params.has_experience);
        if self.growing {
            engine::allocation::reserve(&mut self.vectors[table], 1)
                .map_err(|_| "character trait vector allocation failed")?;
        }
        if self.vectors[table].len() == self.vectors[table].capacity() {
            self.overflowed = true;
            return Err("character trait vector staging capacity exhausted");
        }
        let mut vector = TraitVector {
            unit: params.unit,
            leader: params.leader_id,
            start: self.occurrences[table].len(),
            len: 0,
            outcome: Outcome::Rows { len: 0 },
            bytes_len: 0,
            required_bytes: 0,
        };
        let result = (|| -> Result<(), &'static str> {
            let identity = read::<[u8; 16]>(memory, params.unit).ok_or_else(|| {
                self.fault[table] = true;
                "unit leader identity unreadable"
            })?;
            self.raw_bytes[table] += 16;
            let relative = usize::try_from(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                .ok()
                .and_then(|value| value.checked_sub(base));
            if !matches!(
                relative,
                Some(
                    cunit_leader::VTABLE_RVA
                        | carmy_leader::VTABLE_RVA
                        | cnavy_leader::VTABLE_RVA
                        | coperative_leader::VTABLE_RVA
                )
            ) {
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
                .checked_add(params.array_offset)
                .ok_or("unit leader address overflow")?;
            let header = read::<[u8; 16]>(memory, at);
            if header.is_none() {
                self.fault[table] = true;
            } else {
                self.raw_bytes[table] += 16;
            }
            let array = header
                .and_then(|header| Array::from_header(header, params.stride))
                .ok_or("unit leader trait vector invalid or unreadable")?;
            // 140C0D920 bounds these vectors by +0xDD4/+0xE04. Their
            // original acquired counts reserve every occurrence before reads.
            if self.growing {
                engine::allocation::reserve(&mut self.occurrences[table], array.count as usize)
                    .map_err(|_| "character trait occurrence allocation failed")?;
            }
            base.checked_add(cunit_leader_trait::VTABLE_RVA)
                .ok_or("unit leader address overflow")?;
            let row_limit = if self.growing {
                array.count as usize
            } else if self.truncated[table] {
                0
            } else {
                self.rows[table].saturating_sub(self.used_rows[table])
            };
            let byte_limit = if self.truncated[table] {
                0
            } else {
                self.bytes[table].saturating_sub(self.used_bytes[table])
            };
            let mut unread = 0;
            for ordinal in 0..array.count {
                if vector.len == row_limit {
                    unread = array.count - ordinal;
                    break;
                }
                if self.occurrences[table].len() == self.occurrences[table].capacity() {
                    self.overflowed = true;
                    return Err("character trait occurrence staging capacity exhausted");
                }
                let address = array
                    .element(ordinal, params.stride)
                    .ok_or("unit leader trait entry address overflow")?;
                let mut entry = [0u8; 16];
                if !memory.copy(address, &mut entry[..params.stride]) {
                    self.fault[table] = true;
                    return Err(if params.has_experience {
                        "unit leader trait experience entry unreadable"
                    } else {
                        "unit leader trait pointer unreadable"
                    });
                }
                self.raw_bytes[table] += params.stride as u64;
                let address = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                    .map_err(|_| "unit leader trait pointer overflow")?;
                let definition = self.definitions.header(memory, base, address, table)?;
                self.definitions.result(definition)?;
                let len = self.definitions.token_len(definition);
                vector.required_bytes = vector
                    .required_bytes
                    .checked_add(len)
                    .ok_or("unit leader trait byte count overflow")?;
                let fits = self.growing || len <= byte_limit - vector.bytes_len;
                let definition = self.definitions.consume(memory, definition, table, fits)?;
                self.definitions.result(definition)?;
                if !fits {
                    unread = array.count - ordinal;
                    break;
                }
                self.occurrences[table].push(TraitOccurrence {
                    entry,
                    ordinal,
                    definition,
                });
                vector.len += 1;
                vector.bytes_len += len;
            }
            vector.outcome = if unread == 0 {
                Outcome::Rows { len: vector.len }
            } else {
                Outcome::Truncated {
                    len: vector.len,
                    unread_rows: unread,
                }
            };
            self.used_rows[table] += vector.len;
            self.used_bytes[table] += vector.bytes_len;
            self.truncated[table] |= unread != 0;
            Ok(())
        })();
        if let Err(reason) = result {
            vector.outcome = Outcome::Unavailable { reason };
        }
        self.vectors[table].push(vector);
        result
    }

    pub(super) fn finish(&mut self) {
        for table in 0..2 {
            self.fault[table] |= self.definitions.log.fault[table];
            self.raw_bytes[table] += self.definitions.log.raw_bytes[table];
        }
        self.update_charge();
        self.definitions.update_charge();
    }

    pub(super) fn demand(&self) -> [Result<(usize, usize), &'static str>; 2] {
        std::array::from_fn(|table| {
            self.vectors[table]
                .iter()
                .try_fold(0_usize, |bytes, vector| {
                    bytes
                        .checked_add(vector.bytes_len)
                        .ok_or("unit leader trait projection byte count overflow")
                })
                .map(|bytes| (self.occurrences[table].len(), bytes))
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn project<R>(
        &self,
        table: usize,
        cursor: &mut usize,
        unit: usize,
        leader: u32,
        rows: &mut [R],
        bytes: &mut [u8],
        make: impl Fn(cunit_leader::TraitRow, i64) -> R,
    ) -> ReadResult {
        let Some(vector) = self.vectors[table].get(*cursor) else {
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
        if rows.len() < vector.len || bytes.len() < vector.bytes_len {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned projection capacity mismatch",
            });
        }
        let mut start = 0;
        for (output, occurrence) in rows
            .iter_mut()
            .zip(&self.occurrences[table][vector.start..vector.start + vector.len])
        {
            let (token_id, definition_ordinal, token) =
                self.definitions.project(occurrence.definition);
            bytes[start..start + token.len()].copy_from_slice(token);
            *output = make(
                cunit_leader::TraitRow {
                    leader_id: leader,
                    ordinal: occurrence.ordinal,
                    token_id,
                    definition_ordinal,
                    token_start: start,
                    token_len: token.len(),
                },
                i64::from_le_bytes(occurrence.entry[8..].try_into().unwrap()),
            );
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

/// Native trait-vector identity and layout for one owned acquisition.
#[derive(Debug, Clone, Copy)]
pub(super) struct TraitVectorCopy {
    pub(super) unit: usize,
    pub(super) leader_id: u32,
    pub(super) array_offset: usize,
    pub(super) stride: usize,
    pub(super) has_experience: bool,
}
