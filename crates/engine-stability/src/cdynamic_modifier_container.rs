
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native::ccountry;
use engine_native_parts::cdynamic_modifier;
use crate::dynamic_modifier_entry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x0285_3f18);
pub const COUNTRY: usize = 0x08;
pub const STATE: usize = 0x18;
pub const ENTRIES: usize = 0x28;
#[cfg(test)]
pub const ENTRIES_CAPACITY: usize = 0x30;
#[cfg(test)]
pub const ENTRIES_COUNT: usize = 0x34;

pub const ADD_RVA: Rva = Rva(0x0059_e540);
pub const DAILY_UPDATE_RVA: Rva = Rva(0x0059_f8a0);
pub const DAILY_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub scope_country: u32,
    pub scope_state: i32,
    pub days_remaining: i32,
    pub enabled: u8,
    pub token_start: usize,
    pub token_len: usize,
    pub value_start: usize,
    pub value_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub token_bytes: usize,
    pub values_len: usize,
    pub required_token_bytes: usize,
    pub required_values: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
    pub values: usize,
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
) -> Result<Demand, &'static str> {
    let walked = count(memory, base, country, country_id);
    let rows = match walked.outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("dynamic modifier sizing row count overflow")?,
        Outcome::Unavailable { reason } => return Err(reason),
    };
    Ok(Demand {
        rows,
        bytes: walked.required_token_bytes,
        values: walked.required_values,
    })
}

pub fn count(memory: &impl Memory, base: usize, country: usize, country_id: u32) -> Walk {
    walk_into(memory, base, country, country_id, engine_native_memory::walk::Buffer::Count, 0, engine_native_memory::walk::Buffer::Count, 0, engine_native_memory::walk::Buffer::Count, 0)
}

#[allow(clippy::too_many_arguments)]
pub fn walk(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    output: &mut Vec<Row>,
    row_start: usize,
    tokens: &mut Vec<u8>,
    byte_start: usize,
    values: &mut Vec<i64>,
    value_start: usize,
) -> Walk {
    walk_into(
        memory,
        base,
        country,
        country_id,
        engine_native_memory::walk::Buffer::Growing(output),
        row_start,
        engine_native_memory::walk::Buffer::Growing(tokens),
        byte_start,
        engine_native_memory::walk::Buffer::Growing(values),
        value_start,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_into(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    row_start: usize,
    mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
    byte_start: usize,
    mut values: engine_native_memory::walk::Buffer<'_, i64>,
    value_start: usize,
) -> Walk {
    let result = (|| {
        let container = country
            .checked_add(ccountry::DYNAMIC_MODIFIERS)
            .ok_or("dynamic modifier container address overflow")?;
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let identity = read::<[u8; 16]>(memory, container)
            .ok_or("dynamic modifier container identity unreadable")?;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
            || u64::from_le_bytes(identity[8..].try_into().unwrap()) != country as u64
        {
            return Err("dynamic modifier container vtable or country backlink mismatch");
        }
        let array = Array::read(
            memory,
            container
                .checked_add(ENTRIES)
                .ok_or("dynamic modifier array address overflow")?,
            dynamic_modifier_entry::STRIDE,
        )
        .ok_or("dynamic modifier array header unreadable or invalid")?;
        output.ensure(
            row_start
                .checked_add(array.count as usize)
                .ok_or("dynamic modifier row count overflow")?,
        )?;
        let mut len = 0;
        let mut token_bytes = 0_usize;
        let mut values_len = 0_usize;
        let mut required_token_bytes = 0_usize;
        let mut required_values = 0_usize;
        let mut unread_rows = 0_u32;
        for ordinal in 0..array.count {
            let entry = dynamic_modifier_entry::header(
                memory,
                base,
                array
                    .element(ordinal, dynamic_modifier_entry::STRIDE)
                    .ok_or("dynamic modifier entry address overflow")?,
            )?;
            let (token, token_len) = cdynamic_modifier::token(memory, base, entry.definition)?;
            if entry.values.count != cdynamic_modifier::value_count(memory, entry.definition)? {
                return Err("dynamic modifier instance and definition value counts disagree");
            }
            let value_len = entry.values.count as usize;
            required_token_bytes = required_token_bytes
                .checked_add(token_len)
                .ok_or("dynamic modifier token count overflow")?;
            required_values = required_values
                .checked_add(value_len)
                .ok_or("dynamic modifier value count overflow")?;
            tokens.ensure(
                byte_start
                    .checked_add(required_token_bytes)
                    .ok_or("dynamic modifier token count overflow")?,
            )?;
            values.ensure(
                value_start
                    .checked_add(required_values)
                    .ok_or("dynamic modifier value count overflow")?,
            )?;
            if row_start + len == output.len()
                || token_len > tokens.len().saturating_sub(byte_start + token_bytes)
                || value_len > values.len().saturating_sub(value_start + values_len)
            {
                unread_rows += 1;
                continue;
            }
            if !read_slice(
                memory,
                token,
                &mut tokens[byte_start + token_bytes..byte_start + token_bytes + token_len],
            ) {
                return Err("dynamic modifier token bytes unreadable");
            }
            if !dynamic_modifier_entry::copy_values(
                memory,
                entry.values,
                &mut values[value_start + values_len..value_start + values_len + value_len],
            ) {
                return Err("dynamic modifier value bytes unreadable");
            }
            output[row_start + len] = Row {
                country: country_id,
                ordinal,
                scope_country: entry.scope_country,
                scope_state: entry.scope_state,
                days_remaining: entry.days_remaining,
                enabled: entry.enabled,
                token_start: token_bytes,
                token_len,
                value_start: values_len,
                value_len,
            };
            len += 1;
            token_bytes += token_len;
            values_len += value_len;
        }
        Ok(Walk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            token_bytes,
            values_len,
            required_token_bytes,
            required_values,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        token_bytes: 0,
        values_len: 0,
        required_token_bytes: 0,
        required_values: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    const COUNTRY_PTR: usize = 0x1000;

    fn put<const N: usize>(bytes: &mut [u8], address: usize, value: [u8; N]) {
        bytes[address - COUNTRY_PTR..address - COUNTRY_PTR + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        let container = COUNTRY_PTR + ccountry::DYNAMIC_MODIFIERS;
        put(
            &mut bytes,
            container,
            ((BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            container + COUNTRY,
            (COUNTRY_PTR as u64).to_le_bytes(),
        );
        put(&mut bytes, container + ENTRIES, 0x2800_u64.to_le_bytes());
        put(
            &mut bytes,
            container + ENTRIES_CAPACITY,
            2_u32.to_le_bytes(),
        );
        put(&mut bytes, container + ENTRIES_COUNT, 2_u32.to_le_bytes());
        for ordinal in 0..2 {
            let entry = 0x2800 + ordinal * dynamic_modifier_entry::STRIDE;
            let definition = 0x2900;
            put(
                &mut bytes,
                entry,
                ((BASE + dynamic_modifier_entry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::SCOPE_COUNTRY,
                if ordinal == 0 { u32::MAX } else { 8_u32 }.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::SCOPE_STATE,
                if ordinal == 0 { -1_i32 } else { i32::MIN }.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::DAYS_REMAINING,
                if ordinal == 0 { -1_i32 } else { 0_i32 }.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::DEFINITION,
                (definition as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::ENABLED,
                [ordinal as u8],
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::VALUES,
                0x2b00_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::VALUES_CAPACITY,
                2_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + dynamic_modifier_entry::VALUES_COUNT,
                2_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition,
                ((BASE + cdynamic_modifier::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cdynamic_modifier::VALUE_COUNT,
                2_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cdynamic_modifier::ID,
                [b'a', 0xff, 0],
            );
            put(
                &mut bytes,
                definition + cdynamic_modifier::ID + 16,
                2_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cdynamic_modifier::ID + 24,
                15_u64.to_le_bytes(),
            );
        }
        put(&mut bytes, 0x2b00, i64::MIN.to_le_bytes());
        put(&mut bytes, 0x2b08, i64::MAX.to_le_bytes());
        bytes
    }

    #[test]
    fn preserves_disabled_permanent_zero_day_entries_and_full_width_values() {
        let bytes = fixture();
        let mut rows = Vec::new();
        let mut tokens = Vec::new();
        let mut values = Vec::new();
        let result = walk(
            &SliceMemory {
                base: COUNTRY_PTR,
                bytes: &bytes,
            },
            BASE,
            COUNTRY_PTR,
            8,
            &mut rows,
            0,
            &mut tokens,
            0,
            &mut values,
            0,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(tokens, [b'a', 0xff, b'a', 0xff]);
        assert_eq!(values, [i64::MIN, i64::MAX, i64::MIN, i64::MAX]);
        assert_eq!((rows[0].days_remaining, rows[0].enabled), (-1, 0));
        assert_eq!((rows[1].days_remaining, rows[1].enabled), (0, 1));
        assert_eq!((rows[0].scope_country, rows[0].scope_state), (u32::MAX, -1));
        assert_eq!((rows[1].scope_country, rows[1].scope_state), (8, i32::MIN));
    }

    #[test]
    fn bad_empty_container_is_unavailable() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut tokens = Vec::new();
        let mut values = Vec::new();
        put(
            &mut bytes,
            COUNTRY_PTR + ccountry::DYNAMIC_MODIFIERS + ENTRIES_COUNT,
            0_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY_PTR + ccountry::DYNAMIC_MODIFIERS + COUNTRY,
            0_u64.to_le_bytes(),
        );
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: COUNTRY_PTR,
                    bytes: &bytes
                },
                BASE,
                COUNTRY_PTR,
                8,
                &mut rows,
                0,
                &mut tokens,
                0,
                &mut values,
                0
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
