
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a4_68c8);
pub const STRIDE: usize = 0xb0;
pub const COUNTRY: usize = 0x08;
pub const INSTANCES: usize = 0x18;
pub const USED_TYPES: usize = 0x30;
pub const TARGET_COOLDOWNS: usize = 0x48;
pub const TARGET_COOLDOWN_STRIDE: usize = 0x40;
pub const ELIGIBLE_TYPES: usize = 0x68;
pub const COOLDOWN_VTABLE_RVA: Rva = Rva(0x02a4_6878);
pub const COOLDOWN_TYPE: usize = 0x08;
pub const COOLDOWN_SCOPE: usize = 0x10;
pub const COOLDOWN_DAYS: usize = 0x38;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0158_3a50);

pub const POINTER_STRIDE: usize = 8;

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};
use engine_native::ccountry;
use crate::{craid_target, raid_database};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub canonical_slot: u32,
    pub country: i32,
    pub eligible_start: usize,
    pub eligible_len: usize,
    pub used_start: usize,
    pub used_len: usize,
}

pub(crate) struct Children {
    pub instances: Array,
    pub eligible: Array,
    pub used: Array,
    pub cooldowns: Array,
}

pub(crate) fn authenticate_country(
    countries: &ccountry::Reader<'_, impl Memory>,
    raw: i32,
) -> Result<Option<u32>, &'static str> {
    if raw <= 0 {
        return Ok(None);
    }
    Ok(Some(countries.resolve(raw as u32)?.canonical_slot))
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

const PARENT_FIELDS: [RecordField; 5] = [
    field(0, 0, 12),
    field(INSTANCES, 12, 16),
    field(USED_TYPES, 28, 16),
    field(ELIGIBLE_TYPES, 44, 16),
    field(TARGET_COOLDOWNS, 60, 16),
];
pub(crate) const PARENT_BYTES: usize = 76;
pub(crate) const PARENT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PARENT_FIELDS, PARENT_BYTES).expect("valid raid parent layout");

pub(crate) fn copy_parents(
    memory: &impl Memory,
    roster: Array,
    first: u32,
    count: usize,
    output: &mut [u8],
) -> Result<(), &'static str> {
    let base = roster
        .element(first, STRIDE)
        .ok_or("raid parent address overflow")?;
    if memory.copy_records(
        RecordSources::Strided {
            base,
            stride: STRIDE,
            count,
        },
        &PARENT_LAYOUT,
        &mut output[..count * PARENT_BYTES],
    ) < count * PARENT_FIELDS.len()
    {
        return Err("raid parent unreadable");
    }
    Ok(())
}

pub(crate) fn parse_row(
    record: &[u8],
    base: usize,
    ordinal: u32,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<(Row, Children), &'static str> {
    let header = |at: usize| -> [u8; 16] { record[at..at + 16].try_into().unwrap() };
    let identity = &record[..12];
    let (instances, used, eligible, cooldowns) = (header(12), header(28), header(44), header(60));
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("raid parent vtable mismatch");
    }
    let country = i32::from_le_bytes(identity[8..].try_into().unwrap());
    if authenticate_country(countries, country)?.is_some_and(|slot| slot != ordinal) {
        return Err("raid parent country differs from canonical slot");
    }
    let array = |header, what| Array::from_header(header, POINTER_STRIDE).ok_or(what);
    let cooldowns = Array::from_header(cooldowns, TARGET_COOLDOWN_STRIDE)
        .ok_or("raid cooldown array invalid")?;
    Ok((
        Row {
            canonical_slot: ordinal,
            country,
            ..Row::default()
        },
        Children {
            instances: array(instances, "raid instance array invalid")?,
            eligible: array(eligible, "raid eligible-type array invalid")?,
            used: array(used, "raid used-type array invalid")?,
            cooldowns,
        },
    ))
}

pub(crate) fn type_token(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<u32, &'static str> {
    let vtable = engine_sites::at(base, raid_database::TYPE_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let identity = read::<[u8; raid_database::TYPE_TOKEN + 4]>(memory, definition)
        .ok_or("raid type unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != vtable {
        return Err("raid type vtable mismatch");
    }
    Ok(u32::from_le_bytes(
        identity[raid_database::TYPE_TOKEN..].try_into().unwrap(),
    ))
}

pub struct TypeTokens {
    slots: Vec<(usize, u32)>,
    len: usize,
    required: usize,
}

impl TypeTokens {
    #[cfg(any(test, feature = "test-support"))]
    pub fn try_with_definitions(definitions: usize) -> std::io::Result<Self> {
        Ok(Self {
            slots: engine_allocation::filled(Self::slots_for(definitions)?, (0, 0))?,
            len: 0,
            required: definitions,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_definitions(definitions: usize) -> Self {
        Self::try_with_definitions(definitions).unwrap()
    }

    pub fn empty() -> Self {
        Self {
            slots: Vec::new(),
            len: 0,
            required: 0,
        }
    }

    fn slots_for(definitions: usize) -> std::io::Result<usize> {
        if definitions == 0 {
            return Ok(0);
        }
        definitions
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(engine_allocation::oom)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<(usize, u32)>()
    }

    pub fn grow(&mut self) -> std::io::Result<()> {
        let slots = Self::slots_for(self.required)?;
        if !self.slots.is_empty() && self.slots.len() < slots {
            engine_allocation::resize(&mut self.slots, slots, (0, 0))?;
            self.slots.fill((0, 0));
            self.len = 0;
        }
        Ok(())
    }

    pub(crate) fn begin(&mut self, memory: &impl Memory, base: usize) {
        self.slots.fill((0, 0));
        self.len = 0;
        let count = engine_sites::at(base, raid_database::SINGLETON_RVA)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .and_then(|database| database.checked_add(raid_database::DEFINITIONS_COUNT))
            .and_then(|at| read::<i32>(memory, at))
            .and_then(|count| usize::try_from(count).ok());
        if let Some(count) = count {
            self.required = count;
        }
    }

    pub fn prepare_worker(&mut self) -> Result<(), &'static str> {
        let slots =
            Self::slots_for(self.required).map_err(|_| "capture buffer allocation failed")?;
        if self.slots.len() < slots {
            engine_allocation::resize(&mut self.slots, slots, (0, 0))
                .map_err(|_| "capture buffer allocation failed")?;
            self.len = 0;
        }
        Ok(())
    }

    pub fn begin_with_count(&mut self, count: usize) {
        self.slots.fill((0, 0));
        self.len = 0;
        self.required = count;
    }

    pub fn token(
        &mut self,
        memory: &impl Memory,
        base: usize,
        definition: usize,
    ) -> Result<u32, &'static str> {
        if definition == 0 || self.slots.is_empty() {
            return type_token(memory, base, definition);
        }
        let mask = self.slots.len() - 1;
        let mut index =
            ((definition as u64 >> 3).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize & mask;
        loop {
            match self.slots[index] {
                (address, token) if address == definition => return Ok(token),
                (0, _) => break,
                _ => index = (index + 1) & mask,
            }
        }
        let token = type_token(memory, base, definition)?;
        if self.len + 1 < self.slots.len() {
            self.slots[index] = (definition, token);
            self.len += 1;
        }
        Ok(token)
    }
}

const COOLDOWN_FIELDS: [RecordField; 3] = [
    field(0, 0, 16),
    field(COOLDOWN_SCOPE, 16, craid_target::ARMS_BYTES),
    field(COOLDOWN_DAYS, 16 + craid_target::ARMS_BYTES, 4),
];
pub(crate) const COOLDOWN_BYTES: usize = 20 + craid_target::ARMS_BYTES;
const COOLDOWN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COOLDOWN_FIELDS, COOLDOWN_BYTES).expect("valid raid cooldown layout");

pub(crate) fn copy_cooldowns(
    memory: &impl Memory,
    cooldowns: Array,
    first: u32,
    count: usize,
    output: &mut [u8],
) -> Result<(), &'static str> {
    let base = cooldowns
        .element(first, TARGET_COOLDOWN_STRIDE)
        .ok_or("raid cooldown address overflow")?;
    if memory.copy_records(
        RecordSources::Strided {
            base,
            stride: TARGET_COOLDOWN_STRIDE,
            count,
        },
        &COOLDOWN_LAYOUT,
        &mut output[..count * COOLDOWN_BYTES],
    ) < count * COOLDOWN_FIELDS.len()
    {
        return Err("raid cooldown unreadable");
    }
    Ok(())
}

pub(crate) fn parse_cooldown(
    memory: &impl Memory,
    base: usize,
    record: &[u8],
    tokens: &mut TypeTokens,
) -> Result<(u32, craid_target::Row, i32), &'static str> {
    let vtable = engine_sites::at(base, COOLDOWN_VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable as u64 {
        return Err("raid cooldown vtable mismatch");
    }
    let definition = usize::try_from(u64::from_le_bytes(record[8..16].try_into().unwrap()))
        .ok()
        .filter(|&address| address != 0)
        .ok_or("raid cooldown type null")?;
    Ok((
        tokens.token(memory, base, definition)?,
        craid_target::row_from_arms(
            memory,
            base,
            record[16..COOLDOWN_BYTES - 4].try_into().unwrap(),
        )?,
        i32::from_le_bytes(
            record[COOLDOWN_BYTES - 4..COOLDOWN_BYTES]
                .try_into()
                .unwrap(),
        ),
    ))
}

pub(crate) fn copy_type_tokens(
    memory: &impl Memory,
    base: usize,
    source: Array,
    output: &mut [u32],
    tokens: &mut TypeTokens,
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    let mut pointers = [0_u8; BATCH * POINTER_STRIDE];
    for (chunk, first) in output.chunks_mut(BATCH).zip((0..).step_by(BATCH)) {
        let bytes = &mut pointers[..chunk.len() * POINTER_STRIDE];
        let address = source
            .element(first as u32, POINTER_STRIDE)
            .ok_or("raid type pointer address overflow")?;
        if !engine_native_memory::memory::read_slice(memory, address, bytes) {
            return Err("raid type pointers unreadable");
        }
        for (token, pointer) in chunk.iter_mut().zip(bytes.chunks_exact(POINTER_STRIDE)) {
            let definition = usize::try_from(u64::from_le_bytes(pointer.try_into().unwrap()))
                .ok()
                .filter(|&address| address != 0)
                .ok_or("raid type pointer unreadable or null")?;
            *token = tokens.token(memory, base, definition)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definitions(count: usize) -> (Vec<u8>, Vec<usize>) {
        const FIRST: usize = 0x100;
        const STRIDE: usize = 0x20;
        let mut bytes = vec![0_u8; FIRST + count * STRIDE];
        let addresses: Vec<usize> = (0..count).map(|index| FIRST + index * STRIDE).collect();
        for (index, &at) in addresses.iter().enumerate() {
            bytes[at..at + 8]
                .copy_from_slice(&(raid_database::TYPE_VTABLE_RVA.0 as u64).to_le_bytes());
            bytes[at + raid_database::TYPE_TOKEN..at + raid_database::TYPE_TOKEN + 4]
                .copy_from_slice(&(0x3000 + index as u32).to_le_bytes());
        }
        (bytes, addresses)
    }

    #[test]
    fn the_token_memo_holds_every_definition_it_is_sized_for_and_reads_the_rest() {
        for count in [36, 300] {
            let (bytes, addresses) = definitions(count);
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut small = TypeTokens::with_definitions(4);
            let mut sized = TypeTokens::with_definitions(count);
            for round in 0..2 {
                for (index, &definition) in addresses.iter().enumerate() {
                    let expected = 0x3000 + index as u32;
                    assert_eq!(small.token(&memory, 0, definition), Ok(expected));
                    assert_eq!(sized.token(&memory, 0, definition), Ok(expected));
                }
                if round == 0 {
                    assert_eq!(sized.len, count, "every definition is held");
                    assert!(small.len < small.slots.len());
                }
            }
            small.required = count;
            small.grow().unwrap();
            for &definition in &addresses {
                small.token(&memory, 0, definition).unwrap();
            }
            assert_eq!(small.len, count);
        }
    }

}
