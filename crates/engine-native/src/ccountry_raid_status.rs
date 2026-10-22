//! Country raid status stored inline by canonical country slot. The stored
//! country handle may alias another slot; serialization treats signed
//! nonpositive handles as absent.

/// Raid-status vtable.
pub const VTABLE_RVA: usize = 0x029b_6198;
/// Inline raid-status array stride.
pub const STRIDE: usize = 0xb0;
/// Country handle, i32, serialized only when positive.
pub const COUNTRY: usize = 0x08;
/// Raid-instance pointer-array data.
pub const INSTANCES: usize = 0x18;
/// Used-type pointer-array data; serialization resolves definition names.
pub const USED_TYPES: usize = 0x30;
/// Inline target-cooldown array data.
pub const TARGET_COOLDOWNS: usize = 0x48;
/// Inline cooldown stride. Offers compare raid type and the first four scope
/// words; stored days supply tooltip text only.
pub const TARGET_COOLDOWN_STRIDE: usize = 0x40;
/// Country-eligible raid-type cache.
pub const ELIGIBLE_TYPES: usize = 0x68;
/// Target-cooldown vtable.
pub const COOLDOWN_VTABLE_RVA: usize = 0x029b_6148;
/// Cooldown entry: raid type pointer.
pub const COOLDOWN_TYPE: usize = 0x08;
/// Cooldown entry: target scope, as `raid_location_scope_layout`.
pub const COOLDOWN_SCOPE: usize = 0x10;
/// Stored cooldown days used by serialization and denial tooltip.
pub const COOLDOWN_DAYS: usize = 0x38;
/// Raid-status constructor.
pub const CONSTRUCTOR_RVA: usize = 0x014d_6dc0;

/// Pointer stride of the instance, used-type and eligible-type arrays.
pub const POINTER_STRIDE: usize = 8;

use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    craid_target,
    memory::{RecordField, RecordLayout, RecordSources},
    raid_database, read,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Array ordinal, which the engine indexes with the canonical country slot.
    pub canonical_slot: u32,
    /// The stored native handle. Signed nonpositive values mean absence.
    pub country: i32,
    /// [`ELIGIBLE_TYPES`] then [`USED_TYPES`] as raid type tokens, in the
    /// caller's token arena.
    pub eligible_start: usize,
    pub eligible_len: usize,
    pub used_start: usize,
    pub used_len: usize,
}

/// A status's three pointer arrays, validated as it is read.
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

/// A parent's identity and its four child array headers, as one record.
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

/// Copy `count` consecutive parents of `roster` from `first` in one guarded
/// call.
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

/// Decode one parent record [`copy_parents`] copied.
pub(crate) fn parse_row(
    record: &[u8],
    base: usize,
    ordinal: u32,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<(Row, Children), &'static str> {
    let header = |at: usize| -> [u8; 16] { record[at..at + 16].try_into().unwrap() };
    let identity = &record[..12];
    let (instances, used, eligible, cooldowns) = (header(12), header(28), header(44), header(60));
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
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
    let vtable = base
        .checked_add(raid_database::TYPE_VTABLE_RVA)
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

/// Raid type tokens already read in one walk, by definition address. The
/// raid database holds one definition per type and never moves it during a
/// capture, so each is authenticated and read once per walk.
///
/// An open-addressed table the caller owns and sizes from the database's
/// definition count (`TypeTokens::required`, read at `TypeTokens::begin`),
/// growing it before worker lookups or off the walk ([`TypeTokens::grow`]); no count is capped. While it
/// is smaller than the count, a definition it cannot hold is read again each
/// time, never dropped.
pub struct TypeTokens {
    /// `(definition address, token)`; address zero is an empty slot. A power
    /// of two, kept at most half full.
    slots: Vec<(usize, u32)>,
    len: usize,
    /// The raid database's definition count at the last walk.
    required: usize,
}

impl TypeTokens {
    /// A table for `definitions` raid types.
    #[cfg(any(test, feature = "test-support"))]
    pub fn try_with_definitions(definitions: usize) -> std::io::Result<Self> {
        Ok(Self {
            slots: crate::allocation::filled(Self::slots_for(definitions)?, (0, 0))?,
            len: 0,
            required: definitions,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_definitions(definitions: usize) -> Self {
        Self::try_with_definitions(definitions).unwrap()
    }

    /// No storage, for a parked placeholder: every token is read uncached.
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
            .ok_or_else(crate::allocation::oom)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<(usize, u32)>()
    }

    /// Off the walk: grow to the last walk's definition count.
    pub fn grow(&mut self) -> std::io::Result<()> {
        let slots = Self::slots_for(self.required)?;
        if !self.slots.is_empty() && self.slots.len() < slots {
            crate::allocation::resize(&mut self.slots, slots, (0, 0))?;
            self.slots.fill((0, 0));
            self.len = 0;
        }
        Ok(())
    }

    /// Start a walk by clearing previous tokens and reading the live definition
    /// count. An unreadable count retains the previous sizing hint; lookups
    /// remain exact.
    pub(crate) fn begin(&mut self, memory: &impl Memory, base: usize) {
        self.slots.fill((0, 0));
        self.len = 0;
        let count = base
            .checked_add(raid_database::SINGLETON_RVA)
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

    /// Worker storage follows the already acquired database count.
    pub fn prepare_worker(&mut self) -> Result<(), &'static str> {
        let slots =
            Self::slots_for(self.required).map_err(|_| "capture buffer allocation failed")?;
        if self.slots.len() < slots {
            crate::allocation::resize(&mut self.slots, slots, (0, 0))
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

/// A cooldown's identity (vtable, type definition) and its scope's arms.
const COOLDOWN_FIELDS: [RecordField; 3] = [
    field(0, 0, 16),
    field(COOLDOWN_SCOPE, 16, craid_target::ARMS_BYTES),
    field(COOLDOWN_DAYS, 16 + craid_target::ARMS_BYTES, 4),
];
pub(crate) const COOLDOWN_BYTES: usize = 20 + craid_target::ARMS_BYTES;
const COOLDOWN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COOLDOWN_FIELDS, COOLDOWN_BYTES).expect("valid raid cooldown layout");

/// Copy `count` consecutive cooldowns from `first` in one guarded call.
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

/// Decode one cooldown record [`copy_cooldowns`] copied: its raid type token
/// and target scope.
pub(crate) fn parse_cooldown(
    memory: &impl Memory,
    base: usize,
    record: &[u8],
    tokens: &mut TypeTokens,
) -> Result<(u32, craid_target::Row, i32), &'static str> {
    let vtable = base
        .checked_add(COOLDOWN_VTABLE_RVA)
        .ok_or("module address overflow")?;
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

/// Copy raid-type pointers as their lexer tokens in guarded batches. The
/// caller sizes output to the array count.
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
        if !crate::memory::read_slice(memory, address, bytes) {
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

    #[test]
    fn cold_worker_memo_keeps_five_thousand_definitions_and_later_database_growth() {
        let mut memo = TypeTokens::empty();
        for count in [1_500, 5_000] {
            let (bytes, addresses) = definitions(count);
            let memory = crate::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            memo.begin_with_count(count);
            memo.prepare_worker().unwrap();
            for (index, &definition) in addresses.iter().enumerate() {
                assert_eq!(
                    memo.token(&memory, 0, definition),
                    Ok(0x3000 + index as u32)
                );
            }
            assert_eq!(memo.len, count);
            let empty = crate::memory::SliceMemory {
                base: 0,
                bytes: &[],
            };
            for (index, &definition) in addresses.iter().enumerate() {
                assert_eq!(memo.token(&empty, 0, definition), Ok(0x3000 + index as u32));
            }
        }
    }

    /// Distinct raid type definitions at 8-byte-aligned addresses, each
    /// authenticated and carrying token `0x3000 + index`.
    fn definitions(count: usize) -> (Vec<u8>, Vec<usize>) {
        const FIRST: usize = 0x100;
        const STRIDE: usize = 0x20;
        let mut bytes = vec![0_u8; FIRST + count * STRIDE];
        let addresses: Vec<usize> = (0..count).map(|index| FIRST + index * STRIDE).collect();
        for (index, &at) in addresses.iter().enumerate() {
            bytes[at..at + 8]
                .copy_from_slice(&(raid_database::TYPE_VTABLE_RVA as u64).to_le_bytes());
            bytes[at + raid_database::TYPE_TOKEN..at + raid_database::TYPE_TOKEN + 4]
                .copy_from_slice(&(0x3000 + index as u32).to_le_bytes());
        }
        (bytes, addresses)
    }

    #[test]
    fn the_token_memo_holds_every_definition_it_is_sized_for_and_reads_the_rest() {
        for count in [36, 300] {
            let (bytes, addresses) = definitions(count);
            let memory = crate::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            // Sized below the count: exact, and the rest is read uncached.
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
            // Grown to the database count, it holds them all.
            small.required = count;
            small.grow().unwrap();
            for &definition in &addresses {
                small.token(&memory, 0, definition).unwrap();
            }
            assert_eq!(small.len, count);
        }
    }

    #[test]
    #[ignore]
    fn bench_type_token_memo_by_definition_count() {
        for count in [36, 300, 2_000] {
            let (bytes, addresses) = definitions(count);
            let memory = crate::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut memo = TypeTokens::with_definitions(count);
            // A walk's lookups: every definition read many times.
            let lookups = 20_000;
            let started = std::time::Instant::now();
            for round in 0..20 {
                memo.slots.fill((0, 0));
                memo.len = 0;
                for lookup in 0..lookups {
                    let definition = addresses[(lookup * 7 + round) % count];
                    std::hint::black_box(memo.token(&memory, 0, definition).unwrap());
                }
            }
            let per_walk = started.elapsed() / 20;
            println!(
                "type token memo {count} definitions, {lookups} lookups per walk: {per_walk:?}"
            );
        }
    }
}
