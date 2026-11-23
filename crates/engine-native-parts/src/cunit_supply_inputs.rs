
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordLayout, RecordSources},
    read,
};

pub const ASKED: usize = 0x38;
pub const RECEIVED: usize = 0x50;
#[cfg(test)]
pub const AIR_DROPS: usize = 0x58;
pub const ORIGINS: usize = 0x60;
pub const FROM_STATE: usize = 0x78;
#[cfg(test)]
pub const DIVISION_COUNT: usize = 0x80;
pub const DIVISION_DEMAND: usize = 0x88;
#[cfg(test)]
pub const AIRPLANE_COUNT: usize = 0x90;
pub const AIRPLANE_DEMAND: usize = 0x98;
#[cfg(any(test, feature = "test-support"))]
pub const SHIP_COUNT: usize = 0xa0;
pub const SHIP_DEMAND: usize = 0xa8;
pub const DISRUPTION: usize = 0xb0;

pub const ORIGIN_STRIDE: usize = 0x30;
pub const ORIGIN_REQUESTED: usize = 0x10;
#[cfg(any(test, feature = "test-support"))]
pub const ORIGIN_RECEIVED: usize = 0x18;
pub const ORIGIN_PROVINCE: usize = 0x28;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub count: i32,
    pub value: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub province: i32,
    pub requested: i64,
    pub received: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    pub disruption: i64,
    pub division_demand: Demand,
    pub airplane_demand: Demand,
    pub ship_demand: Demand,
    pub origins_len: Result<usize, Failure>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            asked: 0,
            received: 0,
            from_state: 0,
            air_drops: 0,
            disruption: 0,
            division_demand: Demand::default(),
            airplane_demand: Demand::default(),
            ship_demand: Demand::default(),
            origins_len: Ok(0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Unreadable,
    InvalidOrigins,
    OriginsCapacity { required: usize },
    OriginsChanged,
    Allocation,
}

impl Failure {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Unreadable => "unit supply inputs unreadable",
            Self::InvalidOrigins => "unit supply origin header invalid",
            Self::OriginsCapacity { .. } => "unit supply origin arena too small",
            Self::OriginsChanged => "unit supply origin header changed",
            Self::Allocation => "unit supply origin allocation failed",
        }
    }
}

const INPUT_SPANS: [(usize, usize); 6] = [
    (ASKED, 8),
    (RECEIVED, 16),
    (FROM_STATE, 12),
    (DIVISION_DEMAND, 12),
    (AIRPLANE_DEMAND, 12),
    (SHIP_DEMAND, 16),
];
const INPUT_FIELDS: [engine_native_memory::memory::RecordField; 6] = engine_native_memory::memory::packed_fields(INPUT_SPANS);
const INPUT_BYTES: usize = engine_native_memory::memory::packed_stride(INPUT_SPANS);
const INPUT_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&INPUT_FIELDS, INPUT_BYTES) {
    Some(layout) => layout,
    None => panic!("valid supply input layout"),
};

const ORIGIN_FIELDS: [engine_native_memory::memory::RecordField; 2] =
    engine_native_memory::memory::packed_fields([(ORIGIN_REQUESTED, 16), (ORIGIN_PROVINCE, 4)]);
const ORIGIN_BYTES: usize = 20;
const ORIGIN_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&ORIGIN_FIELDS, ORIGIN_BYTES) {
    Some(layout) => layout,
    None => panic!("valid supply origin layout"),
};

pub fn capture_buffer(
    memory: &impl Memory,
    pointer: usize,
    origins: engine_native_memory::walk::Buffer<'_, Origin>,
) -> Result<Inputs, Failure> {
    let mut packed = [0; INPUT_BYTES];
    if pointer == 0
        || memory.copy_records(
            RecordSources::Pointers(&[pointer]),
            &INPUT_LAYOUT,
            &mut packed,
        ) != INPUT_FIELDS.len()
    {
        return Err(Failure::Unreadable);
    }
    let origins_len = capture_origins(memory, pointer, origins);
    let i64_at = |at| i64::from_le_bytes(packed[at..at + 8].try_into().unwrap());
    let i32_at = |at| i32::from_le_bytes(packed[at..at + 4].try_into().unwrap());
    Ok(Inputs {
        asked: i64_at(0),
        received: i64_at(8),
        air_drops: i64_at(16),
        from_state: i64_at(24),
        disruption: i64_at(68),
        division_demand: Demand {
            count: i32_at(32),
            value: i64_at(36),
        },
        airplane_demand: Demand {
            count: i32_at(44),
            value: i64_at(48),
        },
        ship_demand: Demand {
            count: i32_at(56),
            value: i64_at(60),
        },
        origins_len,
    })
}

fn capture_origins(
    memory: &impl Memory,
    pointer: usize,
    mut origins: engine_native_memory::walk::Buffer<'_, Origin>,
) -> Result<usize, Failure> {
    let address = pointer.checked_add(ORIGINS).ok_or(Failure::Unreadable)?;
    let header = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    let array = Array::from_header(header, ORIGIN_STRIDE).ok_or(Failure::InvalidOrigins)?;
    let count = array.count as usize;
    origins.ensure(count).map_err(|_| Failure::Allocation)?;
    if count > origins.len() {
        return Err(Failure::OriginsCapacity { required: count });
    }
    const BATCH: usize = 64;
    let mut bytes = [0; BATCH * ORIGIN_BYTES];
    for (batch, destination) in origins[..count].chunks_mut(BATCH).enumerate() {
        let first = batch * BATCH;
        let source = array
            .element(first as u32, ORIGIN_STRIDE)
            .ok_or(Failure::InvalidOrigins)?;
        if memory.copy_records(
            RecordSources::Strided {
                base: source,
                stride: ORIGIN_STRIDE,
                count: destination.len(),
            },
            &ORIGIN_LAYOUT,
            &mut bytes[..destination.len() * ORIGIN_BYTES],
        ) != destination.len() * ORIGIN_FIELDS.len()
        {
            return Err(Failure::Unreadable);
        }
        for (origin, bytes) in destination.iter_mut().zip(bytes.chunks_exact(ORIGIN_BYTES)) {
            *origin = Origin {
                province: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
                requested: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
                received: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            };
        }
    }
    let current = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    if current != header {
        return Err(Failure::OriginsChanged);
    }
    Ok(count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitCapture {
    pub inputs: Result<Inputs, Failure>,
    pub origins_start: usize,
    pub origins_admitted: bool,
    header: [u8; 16],
}

impl Default for UnitCapture {
    fn default() -> Self {
        Self {
            inputs: Err(Failure::Unreadable),
            origins_start: 0,
            origins_admitted: false,
            header: [0; 16],
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OriginsUse {
    pub len: usize,
    pub required: usize,
}

const UNIT_SPANS: [(usize, usize); 7] = [
    (ASKED, 8),
    (RECEIVED, 16),
    (ORIGINS, 16),
    (FROM_STATE, 12),
    (DIVISION_DEMAND, 12),
    (AIRPLANE_DEMAND, 12),
    (SHIP_DEMAND, 16),
];
const UNIT_HEADER: usize = 2;
const UNIT_FIELDS: [engine_native_memory::memory::RecordField; 7] = engine_native_memory::memory::packed_fields(UNIT_SPANS);
const UNIT_BYTES: usize = engine_native_memory::memory::packed_stride(UNIT_SPANS);
const UNIT_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&UNIT_FIELDS, UNIT_BYTES) {
    Some(layout) => layout,
    None => panic!("valid supply unit layout"),
};
const HEADER_FIELDS: [engine_native_memory::memory::RecordField; 1] =
    engine_native_memory::memory::packed_fields([(ORIGINS, 16)]);
const HEADER_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&HEADER_FIELDS, 16) {
    Some(layout) => layout,
    None => panic!("valid supply origin header layout"),
};
const BATCH: usize = 64;

fn unit_scalars(record: &[u8]) -> Inputs {
    let i64_at = |at: usize| i64::from_le_bytes(record[at..at + 8].try_into().unwrap());
    let i32_at = |at: usize| i32::from_le_bytes(record[at..at + 4].try_into().unwrap());
    Inputs {
        asked: i64_at(0),
        received: i64_at(8),
        air_drops: i64_at(16),
        from_state: i64_at(40),
        division_demand: Demand {
            count: i32_at(48),
            value: i64_at(52),
        },
        airplane_demand: Demand {
            count: i32_at(60),
            value: i64_at(64),
        },
        ship_demand: Demand {
            count: i32_at(72),
            value: i64_at(76),
        },
        disruption: i64_at(84),
        origins_len: Ok(0),
    }
}

pub fn capture_units(
    memory: &impl Memory,
    pointer: impl Fn(usize) -> usize,
    units: &mut [UnitCapture],
    origins: &mut Vec<Origin>,
) -> OriginsUse {
    capture_selected_units(memory, pointer, |_| true, units, origins)
}

pub fn capture_selected_units(
    memory: &impl Memory,
    pointer: impl Fn(usize) -> usize,
    selected: impl Fn(usize) -> bool,
    units: &mut [UnitCapture],
    origins: &mut Vec<Origin>,
) -> OriginsUse {
    let mut pointers = [0usize; BATCH];
    let mut indices = [0usize; BATCH];
    let mut records = [0u8; BATCH * UNIT_BYTES];
    for (chunk_index, chunk) in units.chunks_mut(BATCH).enumerate() {
        let first = chunk_index * BATCH;
        let mut count = 0;
        for (row, unit) in chunk.iter_mut().enumerate() {
            *unit = UnitCapture::default();
            if selected(first + row) {
                pointers[count] = pointer(first + row);
                indices[count] = row;
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let bytes = &mut records[..count * UNIT_BYTES];
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &UNIT_LAYOUT,
            bytes,
        );
        for row in 0..count {
            let unit = &mut chunk[indices[row]];
            let unit_pointer = pointers[row];
            let record = &mut bytes[row * UNIT_BYTES..(row + 1) * UNIT_BYTES];
            let mut valid = [false; UNIT_FIELDS.len()];
            for (field, layout) in UNIT_FIELDS.iter().enumerate() {
                let ordinal = row * UNIT_FIELDS.len() + field;
                valid[field] = ordinal < copied
                    || (ordinal > copied
                        && unit_pointer != 0
                        && unit_pointer
                            .checked_add(layout.source_offset)
                            .is_some_and(|at| {
                                memory.copy(
                                    at,
                                    &mut record[layout.destination_offset
                                        ..layout.destination_offset + layout.len],
                                )
                            }));
            }
            *unit = UnitCapture::default();
            let scalars = valid
                .iter()
                .enumerate()
                .all(|(field, valid)| *valid || field == UNIT_HEADER);
            if scalars {
                let mut inputs = unit_scalars(record);
                let header = &UNIT_FIELDS[UNIT_HEADER];
                if valid[UNIT_HEADER] {
                    unit.header = record
                        [header.destination_offset..header.destination_offset + header.len]
                        .try_into()
                        .unwrap();
                } else {
                    inputs.origins_len = Err(Failure::Unreadable);
                }
                unit.inputs = Ok(inputs);
            }
        }
    }

    let mut used = OriginsUse::default();
    for unit in units.iter_mut() {
        let Ok(inputs) = &mut unit.inputs else {
            continue;
        };
        if inputs.origins_len.is_err() {
            continue;
        }
        let Some(array) = Array::from_header(unit.header, ORIGIN_STRIDE) else {
            inputs.origins_len = Err(Failure::InvalidOrigins);
            continue;
        };
        let count = array.count as usize;
        used.required = used.required.saturating_add(count);
        let Some(end) = used.len.checked_add(count) else {
            inputs.origins_len = Err(Failure::OriginsCapacity { required: count });
            continue;
        };
        if end > origins.len() {
            let len = origins.len().saturating_mul(2).max(end);
            if engine_allocation::resize_default(origins, len).is_err() {
                inputs.origins_len = Err(Failure::Allocation);
                continue;
            }
        }
        unit.origins_start = used.len;
        unit.origins_admitted = true;
        used.len = end;
        inputs.origins_len = Ok(count);
    }

    let mut batch = OriginBatch::default();
    for index in 0..units.len() {
        let unit = units[index];
        let count = match unit.inputs {
            Ok(Inputs {
                origins_len: Ok(count),
                ..
            }) if count != 0 => count,
            _ => continue,
        };
        let Some(array) = Array::from_header(unit.header, ORIGIN_STRIDE) else {
            continue;
        };
        for element in 0..count {
            let Some(address) = array.element(element as u32, ORIGIN_STRIDE) else {
                if let Ok(inputs) = &mut units[index].inputs {
                    inputs.origins_len = Err(Failure::InvalidOrigins);
                }
                break;
            };
            batch.push(address, index, unit.origins_start + element);
            if batch.len == BATCH {
                batch.flush(memory, units, origins);
            }
        }
    }
    batch.flush(memory, units, origins);

    let mut indices = [0usize; BATCH];
    let mut len = 0;
    for index in 0..units.len() {
        if matches!(
            units[index].inputs,
            Ok(Inputs {
                origins_len: Ok(count),
                ..
            }) if count != 0
        ) {
            indices[len] = index;
            len += 1;
            if len == BATCH {
                witness_headers(memory, &pointer, &indices[..len], units);
                len = 0;
            }
        }
    }
    witness_headers(memory, &pointer, &indices[..len], units);
    used
}

struct OriginBatch {
    addresses: [usize; BATCH],
    owners: [usize; BATCH],
    slots: [usize; BATCH],
    len: usize,
}

impl Default for OriginBatch {
    fn default() -> Self {
        Self {
            addresses: [0; BATCH],
            owners: [0; BATCH],
            slots: [0; BATCH],
            len: 0,
        }
    }
}

impl OriginBatch {
    fn push(&mut self, address: usize, owner: usize, slot: usize) {
        self.addresses[self.len] = address;
        self.owners[self.len] = owner;
        self.slots[self.len] = slot;
        self.len += 1;
    }

    fn flush(&mut self, memory: &impl Memory, units: &mut [UnitCapture], origins: &mut [Origin]) {
        let mut bytes = [0u8; BATCH * ORIGIN_BYTES];
        let mut done = 0;
        while done < self.len {
            let rest = self.len - done;
            let copied = memory.copy_records(
                RecordSources::Pointers(&self.addresses[done..self.len]),
                &ORIGIN_LAYOUT,
                &mut bytes[..rest * ORIGIN_BYTES],
            );
            let rows = (copied / ORIGIN_FIELDS.len()).min(rest);
            for row in 0..rows {
                let record = &bytes[row * ORIGIN_BYTES..(row + 1) * ORIGIN_BYTES];
                origins[self.slots[done + row]] = Origin {
                    province: i32::from_le_bytes(record[16..20].try_into().unwrap()),
                    requested: i64::from_le_bytes(record[..8].try_into().unwrap()),
                    received: i64::from_le_bytes(record[8..16].try_into().unwrap()),
                };
            }
            done += rows;
            if done < self.len {
                if let Ok(inputs) = &mut units[self.owners[done]].inputs {
                    inputs.origins_len = Err(Failure::Unreadable);
                }
                done += 1;
            }
        }
        self.len = 0;
    }
}

fn witness_headers(
    memory: &impl Memory,
    pointer: &impl Fn(usize) -> usize,
    indices: &[usize],
    units: &mut [UnitCapture],
) {
    if indices.is_empty() {
        return;
    }
    let mut pointers = [0usize; BATCH];
    for (slot, &index) in pointers.iter_mut().zip(indices) {
        *slot = pointer(index);
    }
    let mut headers = [0u8; BATCH * 16];
    let copied = memory.copy_records(
        RecordSources::Pointers(&pointers[..indices.len()]),
        &HEADER_LAYOUT,
        &mut headers[..indices.len() * 16],
    );
    for (row, &index) in indices.iter().enumerate() {
        let header = &mut headers[row * 16..(row + 1) * 16];
        let read = row < copied
            || (row > copied
                && match pointers[row].checked_add(ORIGINS) {
                    Some(at) if pointers[row] != 0 => memory.copy(at, header),
                    _ => false,
                });
        let unit = &mut units[index];
        let failure = if !read {
            Some(Failure::Unreadable)
        } else if header[..] != unit.header[..] {
            Some(Failure::OriginsChanged)
        } else {
            None
        };
        if let (Some(failure), Ok(inputs)) = (failure, &mut unit.inputs) {
            inputs.origins_len = Err(failure);
        }
    }
}

pub fn origin_demand(memory: &impl Memory, pointer: usize) -> Result<usize, Failure> {
    let address = pointer
        .checked_add(ORIGINS)
        .filter(|_| pointer != 0)
        .ok_or(Failure::Unreadable)?;
    let header = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    let array = Array::from_header(header, ORIGIN_STRIDE).ok_or(Failure::InvalidOrigins)?;
    if read::<[u8; 16]>(memory, address) != Some(header) {
        return Err(Failure::OriginsChanged);
    }
    Ok(array.count as usize)
}

#[cfg(test)]
mod tests;
