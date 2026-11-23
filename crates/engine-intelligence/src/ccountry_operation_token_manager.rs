
use engine_sites::Rva;
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::ccountry;
use crate::{operation_token_map::Header, operation_token_target_entry as entry};

pub const VTABLE_RVA: Rva = Rva(0x02a3_9558);
pub const COUNTRY: usize = 0x08;
pub const TARGETS: usize = 0x28;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub target: u32,
    pub tokens_start: usize,
    pub tokens_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub tokens_len: usize,
    pub required_tokens: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    tokens: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(tokens),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut tokens: engine_native_memory::walk::Buffer<'_, u32>,
) -> Walk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let expected = engine_sites::at(base, VTABLE_RVA)
            .ok_or("operation token vtable address overflow")?;
        let empty_storage = engine_sites::at(base, crate::operation_token_map::EMPTY_TARGET_STORAGE_RVA)
            .ok_or("operation token empty storage address overflow")?;
        let mut found_country = false;
        let mut len = 0_usize;
        let mut tokens_len = 0;
        let mut required_tokens = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let manager = country
                .pointer
                .checked_add(ccountry::OPERATION_TOKEN_MANAGER)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("country operation token manager unreadable or null")?;
            let identity = read::<[u8; 16]>(memory, manager)
                .ok_or("operation token manager identity unreadable")?;
            if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
                return Err("operation token manager vtable mismatch");
            }
            if u64::from_le_bytes(identity[COUNTRY..].try_into().unwrap()) != country.pointer as u64
            {
                return Err("operation token manager owner mismatch");
            }
            let map = Header::read(
                memory,
                manager
                    .checked_add(TARGETS)
                    .ok_or("operation token map address overflow")?,
                entry::STRIDE,
                empty_storage,
            )?;
            output.ensure(
                len.checked_add(map.count as usize)
                    .ok_or("operation token row demand overflow")?,
            )?;
            let mut observed = 0_u32;
            for index in 0..map.slots {
                let Some(entry) = entry::read_entry(memory, map.data + index * entry::STRIDE)?
                else {
                    continue;
                };
                let home = (entry.hash & map.mask) as usize;
                if entry.distance > map.overflow || home + usize::from(entry.distance) - 1 != index
                {
                    return Err("operation token map probe distance invalid");
                }
                observed = observed
                    .checked_add(1)
                    .ok_or("operation token map live count overflow")?;
                if observed > map.count {
                    return Err("operation token map census differs from count");
                }
                if entry.target != 0 {
                    countries.resolve(entry.target)?;
                }
                let count = entry.tokens.count as usize;
                required_tokens = required_tokens
                    .checked_add(count)
                    .ok_or("operation token arena requirement overflow")?;
                tokens.ensure(required_tokens)?;
                if len == output.len() || count > tokens.len() - tokens_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("operation token omitted count overflow")?;
                    continue;
                }
                entry::copy_tokens(
                    memory,
                    entry.tokens,
                    &mut tokens[tokens_len..tokens_len + count],
                )?;
                output[len] = Row {
                    owner: country.id,
                    target: entry.target,
                    tokens_start: tokens_len,
                    tokens_len: count,
                };
                len += 1;
                tokens_len += count;
            }
            if observed != map.count {
                return Err("operation token map census differs from count");
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        output[..len].sort_unstable_by_key(|row| (row.owner, row.target));
        if output[..len]
            .windows(2)
            .any(|pair| (pair[0].owner, pair[0].target) == (pair[1].owner, pair[1].target))
        {
            return Err("duplicate operation token target identity");
        }
        Ok(Walk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            tokens_len,
            required_tokens,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        tokens_len: 0,
        required_tokens: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;
    use crate::operation_token_map;

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x1000;
    const FIRST: usize = 0x4000;
    const SECOND: usize = 0x6000;
    const FIRST_MANAGER: usize = 0x8000;
    const SECOND_MANAGER: usize = 0x9000;
    const MAP_DATA: usize = 0xa000;
    const TOKEN_DATA: usize = 0xb000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address..address + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x2000_u64, 4_u32),
            (cgame_state::COUNTRY_LINKS, 0x2100, 4),
        ] {
            put(&mut bytes, GAME + offset, &data.to_le_bytes());
            put(&mut bytes, GAME + offset + 8, &count.to_le_bytes());
            put(&mut bytes, GAME + offset + 12, &count.to_le_bytes());
        }
        for (index, pointer) in [0_u64, FIRST as u64, SECOND as u64, FIRST as u64]
            .into_iter()
            .enumerate()
        {
            put(&mut bytes, 0x2000 + index * 8, &pointer.to_le_bytes());
        }
        for (index, canonical) in [0_u32, 1, 2, 1].into_iter().enumerate() {
            put(&mut bytes, 0x2100 + index * 4, &canonical.to_le_bytes());
        }
        for (country, manager, id) in [
            (FIRST, FIRST_MANAGER, 3_u32),
            (SECOND, SECOND_MANAGER, 2_u32),
        ] {
            put(
                &mut bytes,
                country,
                &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
            put(
                &mut bytes,
                country + ccountry::OPERATION_TOKEN_MANAGER,
                &(manager as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager,
                &((BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + COUNTRY,
                &(country as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + TARGETS + operation_token_map::TABLE,
                &((BASE + operation_token_map::EMPTY_TARGET_STORAGE_RVA.0) as u64).to_le_bytes(),
            );
        }
        let header = FIRST_MANAGER + TARGETS;
        put(
            &mut bytes,
            header + operation_token_map::TABLE,
            &(MAP_DATA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            header + operation_token_map::COUNT,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            header + operation_token_map::MASK,
            &3_u32.to_le_bytes(),
        );
        bytes[header + operation_token_map::OVERFLOW] = 4;
        bytes[MAP_DATA + 8 * entry::STRIDE + entry::OCCUPIED] = 0xff;
        let first_entry = MAP_DATA + entry::STRIDE;
        put(&mut bytes, first_entry + entry::HASH, &1_u32.to_le_bytes());
        bytes[first_entry + entry::OCCUPIED] = 1;
        put(
            &mut bytes,
            first_entry + entry::TARGET_TAG_ID,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            first_entry + entry::TOKENS,
            &(TOKEN_DATA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            first_entry + entry::TOKEN_CAPACITY,
            &160_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            first_entry + entry::TOKEN_COUNT,
            &130_i32.to_le_bytes(),
        );
        for index in 0..130 {
            let token = if index % 2 == 0 {
                u32::MAX
            } else {
                index as u32
            };
            put(&mut bytes, TOKEN_DATA + index * 4, &token.to_le_bytes());
        }
        let overflow_entry = MAP_DATA + 4 * entry::STRIDE;
        put(
            &mut bytes,
            overflow_entry + entry::HASH,
            &3_u32.to_le_bytes(),
        );
        bytes[overflow_entry + entry::OCCUPIED] = 2;
        bytes
    }

    fn capture(bytes: &[u8], output: &mut Vec<Row>, tokens: &mut Vec<u32>) -> Walk {
        walk(&SliceMemory { base: 0, bytes }, BASE, GAME, output, tokens)
    }

    #[test]
    fn growing_targets_and_tokens_use_the_first_and_later_native_counts() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut tokens = Vec::new();
        for count in [130_usize, 5_000] {
            bytes.resize(TOKEN_DATA + count * 4 + 1, 0);
            let at = MAP_DATA + entry::STRIDE;
            put(
                &mut bytes,
                at + entry::TOKEN_CAPACITY,
                &(count as i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + entry::TOKEN_COUNT,
                &(count as i32).to_le_bytes(),
            );
            for index in 0..count {
                put(
                    &mut bytes,
                    TOKEN_DATA + index * 4,
                    &(index as u32).to_le_bytes(),
                );
            }
            let mut expected_rows = vec![Row::default(); 2];
            let mut expected_tokens = vec![0; count];
            let expected = capture(&bytes, &mut expected_rows, &mut expected_tokens);
            assert_eq!(expected.outcome, Outcome::Rows { len: 2 });
            let actual = walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                BASE,
                GAME,
                &mut rows,
                &mut tokens,
            );
            assert_eq!(actual, expected);
            assert_eq!(&rows[..2], &expected_rows);
            assert_eq!(&tokens[..count], expected_tokens.as_slice());
        }
    }

    #[test]
    fn target_entries_preserve_empty_vectors_aliases_and_ordered_repeated_ids() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 4];
        let mut tokens = vec![0; 140];
        let result = capture(&bytes, &mut rows, &mut tokens);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(result.tokens_len, 130);
        assert_eq!(result.required_tokens, 130);
        assert_eq!(
            (rows[0].owner, rows[0].target, rows[0].tokens_len),
            (3, 0, 0)
        );
        assert_eq!(
            (
                rows[1].owner,
                rows[1].target,
                rows[1].tokens_start,
                rows[1].tokens_len
            ),
            (3, 2, 0, 130)
        );
        for (index, &token) in tokens[..130].iter().enumerate() {
            assert_eq!(
                token,
                if index % 2 == 0 {
                    u32::MAX
                } else {
                    index as u32
                }
            );
        }
    }

    #[test]
    fn the_count_probe_reports_exact_requirements() {
        let bytes = fixture();
        let result = count(&SliceMemory { base: 0, bytes: &bytes }, BASE, GAME);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!((result.tokens_len, result.required_tokens), (0, 130));
    }

    #[test]
    fn native_static_empty_map_needs_no_bucket_read_and_must_use_its_sentinel() {
        let mut bytes = fixture();
        let header = FIRST_MANAGER + TARGETS;
        put(
            &mut bytes,
            header + operation_token_map::TABLE,
            &((BASE + operation_token_map::EMPTY_TARGET_STORAGE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, header + operation_token_map::COUNT, &[0; 9]);
        assert_eq!(
            capture(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
        put(
            &mut bytes,
            header + operation_token_map::TABLE,
            &0_u64.to_le_bytes(),
        );
        assert!(matches!(
            capture(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn broken_map_geometry_probe_and_late_payload_fail_the_complete_table() {
        let valid = fixture();
        let mut rows = vec![Row::default(); 4];
        let mut tokens = vec![0; 140];
        for (address, replacement) in [
            (
                FIRST_MANAGER + TARGETS + operation_token_map::COUNT,
                (-1_i32).to_le_bytes().to_vec(),
            ),
            (
                FIRST_MANAGER + TARGETS + operation_token_map::COUNT,
                1_i32.to_le_bytes().to_vec(),
            ),
            (
                FIRST_MANAGER + TARGETS + operation_token_map::MASK,
                5_u32.to_le_bytes().to_vec(),
            ),
            (MAP_DATA + entry::STRIDE + entry::OCCUPIED, vec![0xff]),
            (MAP_DATA + entry::STRIDE + entry::OCCUPIED, vec![2]),
            (MAP_DATA + 8 * entry::STRIDE + entry::OCCUPIED, vec![0]),
            (
                MAP_DATA + entry::STRIDE + entry::TOKEN_COUNT,
                (-1_i32).to_le_bytes().to_vec(),
            ),
            (FIRST_MANAGER + COUNTRY, 0_u64.to_le_bytes().to_vec()),
        ] {
            let mut broken = valid.clone();
            put(&mut broken, address, &replacement);
            let result = capture(&broken, &mut rows, &mut tokens);
            assert!(
                matches!(result.outcome, Outcome::Unavailable { .. }),
                "address {address:x}"
            );
            assert_eq!((result.tokens_len, result.required_tokens), (0, 0));
        }
        let result = capture(&valid[..TOKEN_DATA + 129 * 4 + 3], &mut rows, &mut tokens);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "operation token vector payload unreadable"
            }
        );
    }

    #[test]
    fn duplicate_target_keys_and_unresolvable_countries_are_unavailable() {
        let valid = fixture();
        let mut rows = vec![Row::default(); 4];
        let mut tokens = vec![0; 140];
        let mut duplicate = valid.clone();
        put(
            &mut duplicate,
            MAP_DATA + 4 * entry::STRIDE + entry::TARGET_TAG_ID,
            &2_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&duplicate, &mut rows, &mut tokens).outcome,
            Outcome::Unavailable {
                reason: "duplicate operation token target identity"
            }
        );
        let mut unresolved = valid;
        put(
            &mut unresolved,
            MAP_DATA + entry::STRIDE + entry::TARGET_TAG_ID,
            &4_u32.to_le_bytes(),
        );
        assert!(matches!(
            capture(&unresolved, &mut rows, &mut tokens).outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
