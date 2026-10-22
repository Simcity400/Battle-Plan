//! `CCombatTactic` identity and preferred-tactic modifier key.

use crate::{
    Memory, ccountry, cdynamic_modifier, cnull_combat_tactic, modifier_accumulator,
    modifier_definition, read, read_slice, walk::Outcome,
};

/// Tactic primary vtable.
pub const VTABLE_RVA: usize = 0x027d_0058;
/// Token string.
pub const TOKEN: usize = 0x10;
/// Name string.
pub const NAME: usize = 0x30;
/// Database id, i32.
pub const ID: usize = 0x98;
/// Countered tactic id compared with the opposing tactic id.
pub const COUNTER_ID: usize = 0x90;
/// Modifier-metadata key, u32.
pub const MODIFIER_KEY: usize = 0x174;
/// Tactic-database singleton, lazily created by the game; recording never creates it.
pub const DATABASE_RVA: usize = 0x0331_88c0;
/// Tactic-pointer array in id order, with a null entry at index zero.
pub const DATABASE_ENTRIES: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    /// Zero represents the authenticated CNullCombatTactic object only.
    pub tactic_id: i32,
    pub token_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferred {
    pub outcome: Outcome,
    pub row: Row,
    /// Transient authenticated source pointer for the same callback's weight
    /// read. This address is not replay identity and must not be serialized.
    pub pointer: usize,
    pub required_token_bytes: usize,
}

/// One country's authenticated selection: its tactic object and database ID,
/// `None` for CNullCombatTactic. A null pointer or unknown vtable is an error.
fn selection(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<(usize, Option<i32>), &'static str> {
    let pointer = country
        .checked_add(ccountry::PREFERRED_TACTIC)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or("preferred tactic pointer unreadable or null")?;
    let vtable = read::<u64>(memory, pointer).ok_or("preferred tactic vtable unreadable")?;
    let null_vtable = base
        .checked_add(cnull_combat_tactic::VTABLE_RVA)
        .ok_or("module address overflow")?;
    let selected_vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if vtable == null_vtable as u64 {
        return Ok((pointer, None));
    }
    if vtable != selected_vtable as u64 {
        return Err("preferred tactic vtable mismatch");
    }
    let tactic_id = pointer
        .checked_add(ID)
        .and_then(|p| read::<i32>(memory, p))
        .filter(|id| *id > 0)
        .ok_or("preferred tactic id unreadable or invalid")?;
    Ok((pointer, Some(tactic_id)))
}

/// [`preferred`] without token bytes for consumers needing only selection
/// and weight. Weight evaluation reads no name, so name length cannot
/// make it unavailable.
pub fn preferred_identity(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
) -> Preferred {
    match selection(memory, base, country) {
        Ok((pointer, tactic_id)) => Preferred {
            outcome: Outcome::Rows { len: 1 },
            row: Row {
                country: country_id,
                tactic_id: tactic_id.unwrap_or_default(),
                ..Row::default()
            },
            pointer,
            required_token_bytes: 0,
        },
        Err(reason) => Preferred {
            outcome: Outcome::Unavailable { reason },
            row: Row::default(),
            pointer: 0,
            required_token_bytes: 0,
        },
    }
}

/// Reads one country's selection and its exact token bytes. A null pointer or
/// unknown vtable is unavailable; only CNullCombatTactic means no selection.
pub fn preferred(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    token: &mut [u8],
) -> Preferred {
    preferred_buffer(
        memory,
        base,
        country,
        country_id,
        crate::walk::Buffer::Fixed(token),
    )
}

pub fn preferred_buffer(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    mut token: crate::walk::Buffer<'_, u8>,
) -> Preferred {
    let result = (|| {
        let (pointer, tactic_id) = selection(memory, base, country)?;
        let Some(tactic_id) = tactic_id else {
            return Ok(Preferred {
                outcome: Outcome::Rows { len: 1 },
                row: Row {
                    country: country_id,
                    ..Row::default()
                },
                pointer,
                required_token_bytes: 0,
            });
        };
        let (source, len) = cdynamic_modifier::literal(
            memory,
            pointer
                .checked_add(TOKEN)
                .ok_or("preferred tactic token address overflow")?,
        )?;
        token.ensure(len)?;
        if len > token.len() {
            return Ok(Preferred {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                row: Row::default(),
                pointer,
                required_token_bytes: len,
            });
        }
        if !read_slice(memory, source, &mut token[..len]) {
            return Err("preferred tactic token bytes unreadable");
        }
        Ok(Preferred {
            outcome: Outcome::Rows { len: 1 },
            row: Row {
                country: country_id,
                tactic_id,
                token_len: len,
            },
            pointer,
            required_token_bytes: len,
        })
    })();
    result.unwrap_or_else(|reason| Preferred {
        outcome: Outcome::Unavailable { reason },
        row: Row::default(),
        pointer: 0,
        required_token_bytes: 0,
    })
}

/// Shared immutable metadata context read once per callback, not once per country.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightContext {
    pub country_factor_raw: i64,
    metadata: usize,
}

impl WeightContext {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let country_factor_raw = base
            .checked_add(modifier_definition::COUNTRY_PREFERRED_TACTIC_FACTOR_RVA)
            .and_then(|p| read::<i64>(memory, p))
            .ok_or("preferred tactic country factor unreadable")?;
        let metadata = base
            .checked_add(modifier_definition::TABLE_RVA)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p != 0)
            .ok_or("modifier metadata pointer unreadable or null")?;
        Ok(Self {
            country_factor_raw,
            metadata,
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WeightRow {
    pub country: u32,
    pub tactic_id: i32,
    pub modifier_key: u32,
    pub metadata_flags: u32,
    pub metadata_gate: i32,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub accumulator_entry_present: u8,
}

/// Reads native inputs to the selected-tactic bonus. Derived addition belongs
/// to playback, so the representation retains even values whose sum overflows.
/// A failed weight read does not invalidate the independently read identity.
pub fn weight(
    memory: &impl Memory,
    country: usize,
    preferred: &Preferred,
    context: WeightContext,
) -> Result<Option<WeightRow>, &'static str> {
    if preferred.outcome != (Outcome::Rows { len: 1 }) {
        return Err("preferred tactic identity unavailable for weight");
    }
    if preferred.row.tactic_id == 0 {
        return Ok(None);
    }
    let modifier_key = preferred
        .pointer
        .checked_add(MODIFIER_KEY)
        .and_then(|p| read::<u32>(memory, p))
        .filter(|key| *key != 0)
        .ok_or("preferred tactic modifier key unreadable or zero")?;
    let metadata = (modifier_key as usize)
        .checked_mul(modifier_definition::STRIDE)
        .and_then(|offset| context.metadata.checked_add(offset))
        .ok_or("preferred tactic modifier metadata address overflow")?;
    let fields = metadata
        .checked_add(modifier_definition::FLAGS)
        .and_then(|p| read::<[u8; 12]>(memory, p))
        .ok_or("preferred tactic modifier metadata unreadable")?;
    let metadata_flags = u32::from_le_bytes(fields[..4].try_into().unwrap());
    let metadata_gate = i32::from_le_bytes(fields[8..].try_into().unwrap());
    if metadata_gate != 0 || metadata_flags & modifier_definition::CLAMP_MAXIMUM != 0 {
        return Err("preferred tactic modifier metadata branch unsupported");
    }
    let value = modifier_accumulator::lookup(memory, country, modifier_key)?;
    Ok(Some(WeightRow {
        country: preferred.row.country,
        tactic_id: preferred.row.tactic_id,
        modifier_key,
        metadata_flags,
        metadata_gate,
        country_factor_raw: context.country_factor_raw,
        accumulator_raw: value.raw,
        accumulator_entry_present: u8::from(value.present),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    fn put<const N: usize>(bytes: &mut [u8], address: usize, value: [u8; N]) {
        bytes[address - 0x1000..address - 0x1000 + N].copy_from_slice(&value);
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        put(
            &mut bytes,
            0x1000 + ccountry::PREFERRED_TACTIC,
            0x2800_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2800,
            ((BASE + VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x2800 + ID, 7_i32.to_le_bytes());
        put(&mut bytes, 0x2800 + TOKEN, [b'a', 0xff, 0]);
        put(&mut bytes, 0x2800 + TOKEN + 16, 2_u64.to_le_bytes());
        put(&mut bytes, 0x2800 + TOKEN + 24, 15_u64.to_le_bytes());
        put(&mut bytes, 0x2800 + MODIFIER_KEY, 5_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn authenticated_none_differs_from_fault_and_tokens_are_not_hashed_or_clipped() {
        let mut bytes = fixture();
        let mut token = [0; 2];
        let result = preferred(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            BASE,
            0x1000,
            9,
            &mut token,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.row.tactic_id, 7);
        assert_eq!(token, [b'a', 0xff]);
        assert_eq!(
            preferred(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                BASE,
                0x1000,
                9,
                &mut token[..1]
            )
            .outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        put(
            &mut bytes,
            0x2800,
            ((BASE + cnull_combat_tactic::VTABLE_RVA) as u64).to_le_bytes(),
        );
        let result = preferred(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            BASE,
            0x1000,
            9,
            &mut [],
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.row.tactic_id, 0);
        put(
            &mut bytes,
            0x1000 + ccountry::PREFERRED_TACTIC,
            0_u64.to_le_bytes(),
        );
        assert!(matches!(
            preferred(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                BASE,
                0x1000,
                9,
                &mut token
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }

    /// The identity-only read takes the same selection and ID and never
    /// reads the name, so a name longer than any buffer cannot make it fail,
    /// and it weighs the same as the full read.
    #[test]
    fn identity_ignores_the_name_and_weighs_as_the_full_read() {
        let mut bytes = fixture();
        // A 200-byte heap name at an unmapped address.
        put(&mut bytes, 0x2800 + TOKEN, 0x9000_0000_u64.to_le_bytes());
        put(&mut bytes, 0x2800 + TOKEN + 16, 200_u64.to_le_bytes());
        put(&mut bytes, 0x2800 + TOKEN + 24, 200_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let identity = preferred_identity(&memory, BASE, 0x1000, 9);
        assert_eq!(identity.outcome, Outcome::Rows { len: 1 });
        assert_eq!((identity.row.country, identity.row.tactic_id), (9, 7));
        assert_eq!(identity.pointer, 0x2800);
        // The full read cannot copy that name into its 128-byte buffer.
        assert!(!matches!(
            preferred(&memory, BASE, 0x1000, 9, &mut [0; 128]).outcome,
            Outcome::Rows { .. }
        ));
        let named = fixture();
        let named = SliceMemory {
            base: 0x1000,
            bytes: &named,
        };
        let context = WeightContext {
            country_factor_raw: 1,
            metadata: 0x3000,
        };
        assert_eq!(
            weight(&memory, 0x1000, &identity, context),
            weight(
                &named,
                0x1000,
                &preferred(&named, BASE, 0x1000, 9, &mut [0; 2]),
                context
            )
        );
        put(
            &mut bytes,
            0x1000 + ccountry::PREFERRED_TACTIC,
            0_u64.to_le_bytes(),
        );
        assert!(matches!(
            preferred_identity(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                BASE,
                0x1000,
                9
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn native_weight_inputs_preserve_extremes_and_weight_failure_keeps_identity() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x1000 + ccountry::MODIFIER_ACCUMULATOR,
            0x2b00_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + ccountry::MODIFIER_ACCUMULATOR + 8,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + ccountry::MODIFIER_ACCUMULATOR + 12,
            1_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x2b00, 5_u32.to_le_bytes());
        put(&mut bytes, 0x2b08, i64::MAX.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let preferred = preferred(&memory, BASE, 0x1000, 9, &mut [0; 2]);
        let context = WeightContext {
            country_factor_raw: i64::MAX,
            metadata: 0x3000,
        };
        let row = weight(&memory, 0x1000, &preferred, context)
            .unwrap()
            .unwrap();
        assert_eq!(
            (row.country_factor_raw, row.accumulator_raw),
            (i64::MAX, i64::MAX)
        );
        assert_eq!(row.accumulator_entry_present, 1);
        put(
            &mut bytes,
            0x3000 + 5 * modifier_definition::STRIDE + modifier_definition::FEATURE_SELECTOR,
            1_i32.to_le_bytes(),
        );
        assert!(
            weight(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                &preferred,
                context
            )
            .is_err()
        );
        assert_eq!(preferred.outcome, Outcome::Rows { len: 1 });
    }
}
