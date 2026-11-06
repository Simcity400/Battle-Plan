//! Local-player raid cache supplying allowed raid types for raid offers.
//! The embedded raid-system cache initializes its country to zero and sets
//! the dirty byte. Rebuilding occurs only while dirty: it selects the player
//! and appends allowed raid database indices as u16 values. Updating sets
//! the dirty byte only when the cached country is no longer the player.

use crate::{
    Memory, ccountry_raid_status::TypeTokens, cgame_state, cpdx_array::Array, craid_system,
    raid_database, read, walk::Outcome,
};

/// Embedded raid-system player-status cache.
pub const CACHE: usize = 0xf0;
/// Primary cache vtable.
pub const VTABLE_RVA: usize = 0x02a5_d9f8;
/// Raw country reference this cache was built for; zero before first rebuild.
pub const COUNTRY: usize = 0x08;
/// Allowed raid database indices, u16, with capacity and count.
pub const ALLOWED_TYPES: usize = 0x10;
/// One allowed index.
pub const INDEX_STRIDE: usize = 2;

/// The cache: its country and where its tokens start in the caller's arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub allowed_start: usize,
    pub allowed_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub tokens_len: usize,
    /// Arena demand, including tokens that did not fit.
    pub required_tokens: usize,
}

/// Reads the cache into `rows[0]` and its allowed types, as raid type lexer
/// tokens in native order, into `tokens`. A list longer than `tokens` is a
/// truncation whose demand the caller grows off the walk. No allocation.
/// The player cache's token demand, its allowed list's count: [`walk`]
/// with no storage, which reads only the cache header.
pub fn required_tokens(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let walk = walk(
        memory,
        base,
        game_state,
        &mut [],
        &mut [],
        &mut TypeTokens::empty(),
    );
    match walk.outcome {
        Outcome::Unavailable { reason } => Err(reason),
        _ => Ok(walk.required_tokens),
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [Row],
    tokens: &mut [u32],
    known: &mut TypeTokens,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(rows),
        crate::walk::Buffer::Fixed(tokens),
        known,
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    tokens: &mut Vec<u32>,
    known: &mut TypeTokens,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(rows),
        crate::walk::Buffer::Growing(tokens),
        known,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: crate::walk::Buffer<'_, Row>,
    mut tokens: crate::walk::Buffer<'_, u32>,
    known: &mut TypeTokens,
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        tokens_len: 0,
        required_tokens: 0,
    };
    let result = (|| {
        let root = game_state
            .checked_add(cgame_state::RAID_SYSTEM)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("raid system unreadable or null")?;
        let vtable = |rva: usize| base.checked_add(rva).ok_or("module address overflow");
        if read::<u64>(memory, root) != Some(vtable(craid_system::VTABLE_RVA)? as u64) {
            return Err("raid system vtable mismatch");
        }
        let cache = root
            .checked_add(CACHE)
            .ok_or("player raid cache address overflow")?;
        let header = read::<[u8; ALLOWED_TYPES + 16]>(memory, cache)
            .ok_or("player raid cache unreadable")?;
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != vtable(VTABLE_RVA)? as u64 {
            return Err("player raid cache vtable mismatch");
        }
        let country = u32::from_le_bytes(header[COUNTRY..COUNTRY + 4].try_into().unwrap());
        let allowed = Array::from_header(header[ALLOWED_TYPES..].try_into().unwrap(), INDEX_STRIDE)
            .ok_or("player raid cache allowed list invalid")?;
        let count = allowed.count as usize;
        walk.required_tokens = count;
        rows.ensure(1)?;
        tokens.ensure(count)?;
        let growing = rows.is_growing();
        let (Some(row), Some(output)) = (rows.first_mut(), tokens.get_mut(..count)) else {
            return Ok(Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            });
        };
        let database = base
            .checked_add(raid_database::SINGLETON_RVA)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("raid database unreadable or null")?;
        let definitions = database
            .checked_add(raid_database::DEFINITIONS)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("raid database definitions unreadable")?;
        let defined = database
            .checked_add(raid_database::DEFINITIONS_COUNT)
            .and_then(|at| read::<i32>(memory, at))
            .and_then(|count| usize::try_from(count).ok())
            .ok_or("raid database count unreadable")?;
        known.begin_with_count(defined);
        if growing {
            known.prepare_worker()?;
        }
        const BATCH: usize = 256;
        let mut bytes = [0_u8; BATCH * INDEX_STRIDE];
        for (chunk, slot) in output.chunks_mut(BATCH).enumerate() {
            let bytes = &mut bytes[..slot.len() * INDEX_STRIDE];
            let copied = allowed
                .element((chunk * BATCH) as u32, INDEX_STRIDE)
                .is_some_and(|at| crate::memory::read_slice(memory, at, bytes));
            if !copied {
                return Err("player raid cache allowed list unreadable");
            }
            for (token, index) in slot.iter_mut().zip(bytes.chunks_exact(INDEX_STRIDE)) {
                let index = usize::from(u16::from_le_bytes(index.try_into().unwrap()));
                if index >= defined {
                    return Err("player raid cache index outside the raid database");
                }
                let definition = index
                    .checked_mul(8)
                    .and_then(|offset| definitions.checked_add(offset))
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("raid database definition unreadable or null")?;
                *token = known.token(memory, base, definition)?;
            }
        }
        *row = Row {
            country,
            allowed_start: 0,
            allowed_len: count,
        };
        walk.tokens_len = count;
        Ok(Outcome::Rows { len: 1 })
    })();
    walk.outcome = result.unwrap_or_else(|reason| {
        walk.tokens_len = 0;
        Outcome::Unavailable { reason }
    });
    walk
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const GS: usize = 0x1000;
    const ROOT: usize = 0x2000;
    const INDICES: usize = 0x3000;
    const DATABASE: usize = 0x4000;
    const DEFINITIONS: usize = 0x4800;
    const TYPE: usize = 0x5000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address..address + value.len()].copy_from_slice(value);
    }

    /// A cache built for country 7 allowing database indices 2 then 0.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; raid_database::SINGLETON_RVA + 16];
        put(
            &mut bytes,
            GS + cgame_state::RAID_SYSTEM,
            &(ROOT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            ROOT,
            &(craid_system::VTABLE_RVA as u64).to_le_bytes(),
        );
        let cache = ROOT + CACHE;
        put(&mut bytes, cache, &(VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, cache + COUNTRY, &7_u32.to_le_bytes());
        put(
            &mut bytes,
            cache + ALLOWED_TYPES,
            &(INDICES as u64).to_le_bytes(),
        );
        put(&mut bytes, cache + ALLOWED_TYPES + 8, &4_u32.to_le_bytes());
        put(&mut bytes, cache + ALLOWED_TYPES + 12, &2_u32.to_le_bytes());
        put(&mut bytes, INDICES, &2_u16.to_le_bytes());
        put(&mut bytes, INDICES + 2, &0_u16.to_le_bytes());
        put(
            &mut bytes,
            raid_database::SINGLETON_RVA,
            &(DATABASE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DATABASE + raid_database::DEFINITIONS,
            &(DEFINITIONS as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DATABASE + raid_database::DEFINITIONS_COUNT,
            &3_i32.to_le_bytes(),
        );
        for index in 0..3 {
            let definition = TYPE + index * 0x20;
            put(
                &mut bytes,
                DEFINITIONS + index * 8,
                &(definition as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                definition,
                &(raid_database::TYPE_VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + raid_database::TYPE_TOKEN,
                &(0x100 + index as u32).to_le_bytes(),
            );
        }
        bytes
    }

    fn run(bytes: &[u8], tokens: &mut [u32]) -> (Walk, Row) {
        let memory = SliceMemory { base: 0, bytes };
        let mut rows = [Row::default()];
        let mut known = TypeTokens::with_definitions(4);
        let walk = walk(&memory, 0, GS, &mut rows, tokens, &mut known);
        (walk, rows[0])
    }

    #[test]
    fn the_cache_country_and_its_allowed_types_read_as_tokens_in_native_order() {
        let mut tokens = [0; 4];
        let (walk, row) = run(&fixture(), &mut tokens);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        assert_eq!(row.country, 7);
        assert_eq!(&tokens[..row.allowed_len], &[0x102, 0x100]);
    }

    #[test]
    fn the_token_demand_is_the_allowed_count_without_reading_the_list() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(required_tokens(&memory, 0, GS), Ok(2));
    }

    #[test]
    fn a_short_arena_truncates_and_asks_for_the_list() {
        let mut tokens = [0; 1];
        let (walk, _) = run(&fixture(), &mut tokens);
        assert!(matches!(
            walk.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        ));
        assert_eq!(walk.required_tokens, 2);
    }

    #[test]
    fn a_foreign_cache_or_index_is_refused() {
        let mut tokens = [0; 4];
        let mut bytes = fixture();
        put(&mut bytes, ROOT + CACHE, &0_u64.to_le_bytes());
        assert!(matches!(
            run(&bytes, &mut tokens).0.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut bytes = fixture();
        put(&mut bytes, INDICES, &3_u16.to_le_bytes());
        assert!(matches!(
            run(&bytes, &mut tokens).0.outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
