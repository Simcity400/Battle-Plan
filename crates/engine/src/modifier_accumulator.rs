//! Inline sorted modifier accumulator at CCountry+0x5b8.
//! These readers do not establish the concrete C++ template/class name.

use crate::{Memory, ccountry, cpdx_array::Array, modifier_entry, read};

/// Modifier accumulator data pointer; old build_profile.rs:98 and compact_contract/tactic_weight.rs:23.
pub const DATA: usize = 0x00;
/// Modifier accumulator capacity u32; old compact_contract/tactic_weight.rs:24 and tactic-weight reader.
pub const CAPACITY: usize = 0x08;
/// Modifier accumulator count; old D57/D74 read i32 and tactic-weight reads u32 at the same +0xc word.
pub const COUNT: usize = 0x0c;
pub const VALUE_RVA: usize = 0x0055_1820;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Value {
    pub raw: i64,
    pub present: bool,
}

/// Resolves several strictly increasing keys in one census of the native sorted
/// accumulator. Raw values are never clamped or rescaled; callers apply the
/// metadata branch only when they need the engine's derived effective value.
/// The country pointer must come from the authenticated country census.
pub fn lookup_many(
    memory: &impl Memory,
    country: usize,
    sorted_keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    if output.len() != sorted_keys.len()
        || sorted_keys.first() == Some(&0)
        || sorted_keys.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("modifier lookup keys or output invalid");
    }
    output.fill(Value::default());
    if sorted_keys.is_empty() {
        return Ok(());
    }
    let address = country
        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
        .ok_or("modifier accumulator address overflow")?;
    let array = Array::read(memory, address, modifier_entry::STRIDE)
        .ok_or("modifier accumulator header unreadable or invalid")?;
    let mut previous_key = 0;
    let mut wanted = 0;
    for index in 0..array.count {
        let address = array
            .element(index, modifier_entry::STRIDE)
            .ok_or("modifier accumulator entry address overflow")?;
        let bytes =
            read::<[u8; 16]>(memory, address).ok_or("modifier accumulator entry unreadable")?;
        let key = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        if key <= previous_key {
            return Err("modifier accumulator keys not positive and increasing");
        }
        previous_key = key;
        while wanted < sorted_keys.len() && sorted_keys[wanted] < key {
            wanted += 1;
        }
        if wanted < sorted_keys.len() && sorted_keys[wanted] == key {
            output[wanted] = Value {
                raw: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
                present: true,
            };
            wanted += 1;
        }
    }
    Ok(())
}

pub fn lookup(memory: &impl Memory, country: usize, key: u32) -> Result<Value, &'static str> {
    let mut values = [Value::default()];
    lookup_many(memory, country, &[key], &mut values)?;
    Ok(values[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn resolves_multiple_keys_exactly_and_checks_the_unselected_suffix() {
        let mut bytes = vec![0; 0x700];
        bytes[ccountry::MODIFIER_ACCUMULATOR..ccountry::MODIFIER_ACCUMULATOR + 8]
            .copy_from_slice(&0x1600_u64.to_le_bytes());
        bytes[ccountry::MODIFIER_ACCUMULATOR + 8..ccountry::MODIFIER_ACCUMULATOR + 12]
            .copy_from_slice(&3_u32.to_le_bytes());
        bytes[ccountry::MODIFIER_ACCUMULATOR + 12..ccountry::MODIFIER_ACCUMULATOR + 16]
            .copy_from_slice(&3_u32.to_le_bytes());
        for (index, (key, raw)) in [(2_u32, i64::MIN), (5, 0), (9, i64::MAX)]
            .into_iter()
            .enumerate()
        {
            let at = 0x600 + index * 16;
            bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
        }
        let mut output = [Value::default(); 4];
        lookup_many(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            &[2, 4, 5, 9],
            &mut output,
        )
        .unwrap();
        assert_eq!(
            output,
            [
                Value {
                    raw: i64::MIN,
                    present: true
                },
                Value::default(),
                Value {
                    raw: 0,
                    present: true
                },
                Value {
                    raw: i64::MAX,
                    present: true
                }
            ]
        );
        bytes[0x620..0x624].copy_from_slice(&5_u32.to_le_bytes());
        assert!(
            lookup(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                2
            )
            .is_err()
        );
    }
}
