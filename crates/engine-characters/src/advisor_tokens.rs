//! The character manager's advisor idea-token set holds string keys and their
//! hashes. Idea lookup consults it first to identify advisor names.
//!
//! Buckets are selected by the hash and mask. Probing stops when the stored
//! distance is shorter than the probe length; matching checks the stored hash
//! and then the key. The empty table uses image storage. Rehashing allocates
//! buckets for the mask, maximum probe and end sentinel; erasure and insertion
//! update the live count as advisor roles change.
//!
//! Bucket order follows hashes and insertion history. Consumers sort the keys;
//! the collector publishes them sorted.

use crate::{
    Memory,
    ccharacter::{Literal, NameWalk},
    ccharacter_manager, cgame_state, read, read_slice,
    walk::Outcome,
};

/// The table inside CCharacterManager.
pub const TABLE: usize = 0x60;
/// Bucket array pointer, relative to [`TABLE`].
pub const BUCKETS: usize = 0x08;
/// Live key count, u32, relative to [`TABLE`].
pub const COUNT: usize = 0x10;
/// Bucket mask (bucket count minus one), u32, relative to [`TABLE`].
pub const MASK: usize = 0x14;
/// Maximum probe distance, u8, relative to [`TABLE`].
pub const MAXIMUM_PROBE: usize = 0x18;
/// The image's empty bucket array the constructor installs.
pub const EMPTY_BUCKETS_RVA: usize = 0x030f_d240;
/// Bucket stride.
pub const STRIDE: usize = 0x30;
/// Distance-plus-one byte: zero is empty, 0xFF the end bucket.
pub const DISTANCE: usize = 0x04;
/// The key's `std::string`.
pub const KEY: usize = 0x08;

const EMPTY: u8 = 0;
const END: u8 = 0xff;
/// Buckets copied per guarded read.
const BUCKETS_PER_READ: usize = 16;

/// One key, as a span of the caller's byte arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub len: usize,
}

/// Copies every key of the set into `bytes`, one row per key, in bucket
/// order. A key that does not fit (either buffer) is counted, and its
/// validated length added to `required_bytes`, so empty buffers yield the
/// live demand. A bucket count that disagrees with the header, an end
/// bucket inside the span, or an unreadable key fails the table.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    bytes: &mut [u8],
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(bytes),
    )
}

/// The table header [`walk_carried`] gathers before reading any bucket.
pub const HEADER: usize = 0x19;

/// A caller's held token rows ([`walk_carried`]).
pub trait TokenCarry {
    /// Registers the authenticated manager before any bucket is read; when
    /// the held table may be carried, its row and byte counts.
    fn plan(&mut self, manager: usize, header: &[u8; HEADER]) -> Option<(usize, usize)>;
    /// Writes the carryable held rows and bytes.
    fn carry(&mut self, rows: &mut [Row], bytes: &mut [u8]);
    /// The table read afresh and whole.
    fn read(&mut self, rows: &[Row], bytes: &[u8]);
}

/// No held rows: the table is read.
pub struct Fresh;
impl TokenCarry for Fresh {
    fn plan(&mut self, _: usize, _: &[u8; HEADER]) -> Option<(usize, usize)> {
        None
    }
    fn carry(&mut self, _: &mut [Row], _: &mut [u8]) {}
    fn read(&mut self, _: &[Row], _: &[u8]) {}
}

/// [`walk_growing`] with `carry` supplying the held table it proves
/// current: the manager and table header are still read and authenticated,
/// and a carried table's buckets and keys are not.
pub fn walk_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
    carry: &mut impl TokenCarry,
) -> NameWalk {
    walk_inner(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
        carry,
    )
    .unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// Worker storage grows from the original native headers before publication.
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    bytes: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    walk_inner(memory, base, game_state, output, bytes, &mut Fresh).unwrap_or_else(|reason| {
        NameWalk {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        }
    })
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut bytes: crate::walk::Buffer<'_, u8>,
    carry: &mut impl TokenCarry,
) -> Result<NameWalk, &'static str> {
    let manager = game_state
        .checked_add(cgame_state::CHARACTER_MANAGER)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("character manager unreadable or null")?;
    let vtable = base
        .checked_add(ccharacter_manager::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, manager) != Some(vtable as u64) {
        return Err("character manager vtable mismatch");
    }
    let table = manager
        .checked_add(TABLE)
        .ok_or("advisor token table address overflow")?;
    let header = read::<[u8; HEADER]>(memory, table).ok_or("advisor token table unreadable")?;
    let word = |offset: usize| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    let buckets = usize::try_from(u64::from_le_bytes(
        header[BUCKETS..BUCKETS + 8].try_into().unwrap(),
    ))
    .map_err(|_| "advisor token buckets overflow")?;
    let count = word(COUNT);
    let mask = word(MASK);
    let span = (mask as usize)
        .checked_add(usize::from(header[MAXIMUM_PROBE]))
        .and_then(|span| span.checked_add(1))
        .ok_or("advisor token table span overflow")?;
    if buckets == 0 {
        return Err("advisor token table has null buckets");
    }
    if count > mask.saturating_add(1) {
        return Err("advisor token count exceeds its buckets");
    }
    let empty = base
        .checked_add(EMPTY_BUCKETS_RVA)
        .ok_or("module address overflow")?;
    if buckets == empty {
        // The constructor's image buckets hold no key.
        return if count == 0 {
            Ok(NameWalk {
                outcome: Outcome::Rows { len: 0 },
                bytes_len: 0,
                required_bytes: 0,
            })
        } else {
            Err("advisor token table counts keys in the empty buckets")
        };
    }
    if let Some((rows, needed)) = carry.plan(manager, &header)
        && rows == count as usize
        && output.ensure(rows)?
        && bytes.ensure(needed)?
    {
        carry.carry(&mut output[..rows], &mut bytes[..needed]);
        return Ok(NameWalk {
            outcome: Outcome::Rows { len: rows },
            bytes_len: needed,
            required_bytes: needed,
        });
    }
    let mut len = 0;
    let mut bytes_len = 0_usize;
    let mut required_bytes = 0_usize;
    let mut unread_rows = 0_u32;
    let mut live = 0_u32;
    let mut batch = [0_u8; BUCKETS_PER_READ * STRIDE];
    let mut first = 0;
    while first < span {
        let n = (span - first).min(BUCKETS_PER_READ);
        let address = first
            .checked_mul(STRIDE)
            .and_then(|offset| buckets.checked_add(offset))
            .ok_or("advisor token bucket address overflow")?;
        if !read_slice(memory, address, &mut batch[..n * STRIDE]) {
            return Err("advisor token buckets unreadable");
        }
        for (index, bucket) in batch[..n * STRIDE].chunks_exact(STRIDE).enumerate() {
            match bucket[DISTANCE] {
                EMPTY => continue,
                END => return Err("advisor token end bucket inside the table"),
                _ => {}
            }
            live = live
                .checked_add(1)
                .filter(|&live| live <= count)
                .ok_or("advisor token buckets exceed the count")?;
            let key = address + index * STRIDE + KEY;
            let literal = Literal::from_header(key, bucket[KEY..KEY + 32].try_into().unwrap())?;
            Literal::validate(memory, [&literal])?;
            let end = bytes_len
                .checked_add(literal.len)
                .ok_or("advisor token byte count overflow")?;
            output.ensure(count as usize)?;
            bytes.ensure(end)?;
            if unread_rows != 0 || len == output.len() || end > bytes.len() {
                unread_rows += 1;
                required_bytes = required_bytes
                    .checked_add(literal.len)
                    .ok_or("advisor token byte count overflow")?;
                continue;
            }
            Literal::copy_all(memory, [&literal], &mut bytes[bytes_len..end])?;
            output[len] = Row {
                start: bytes_len,
                len: literal.len,
            };
            len += 1;
            bytes_len = end;
        }
        first += n;
    }
    if live != count {
        return Err("advisor token buckets differ from the count");
    }
    if unread_rows == 0 {
        carry.read(&output[..len], &bytes[..bytes_len]);
    }
    Ok(NameWalk {
        outcome: if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        },
        bytes_len,
        required_bytes: required_bytes
            .checked_add(bytes_len)
            .ok_or("advisor token byte count overflow")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const GS: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const BUCKET_ARRAY: usize = 0x3000;
    const HEAP: usize = 0x5000;
    const LONG: &[u8] = b"GER_hjalmar_schacht_idea";

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn string(bytes: &mut [u8], at: usize, value: &[u8], heap: usize) {
        if value.len() <= 15 {
            bytes[at..at + value.len()].copy_from_slice(value);
            bytes[at + value.len()] = 0;
            p64(bytes, at + 24, 15);
        } else {
            p64(bytes, at, heap as u64);
            bytes[heap..heap + value.len()].copy_from_slice(value);
            bytes[heap + value.len()] = 0;
            p64(bytes, at + 24, 31);
        }
        p64(bytes, at + 16, value.len() as u64);
    }

    /// Eight buckets (mask 7, maximum probe 1): keys in buckets 1, 4 and 8,
    /// the end bucket at 9.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        p64(
            &mut bytes,
            GS + cgame_state::CHARACTER_MANAGER,
            MANAGER as u64,
        );
        p64(&mut bytes, MANAGER, ccharacter_manager::VTABLE_RVA as u64);
        let table = MANAGER + TABLE;
        p64(&mut bytes, table + BUCKETS, BUCKET_ARRAY as u64);
        p32(&mut bytes, table + COUNT, 3);
        p32(&mut bytes, table + MASK, 7);
        bytes[table + MAXIMUM_PROBE] = 1;
        for (index, key) in [(1, b"b_idea".as_slice()), (4, LONG), (8, b"a_idea")] {
            let bucket = BUCKET_ARRAY + index * STRIDE;
            bytes[bucket + DISTANCE] = 1;
            string(&mut bytes, bucket + KEY, key, HEAP);
        }
        bytes[BUCKET_ARRAY + 9 * STRIDE + DISTANCE] = END;
        bytes
    }

    fn run(bytes: &[u8], rows: usize, text: usize) -> (NameWalk, Vec<Row>, Vec<u8>) {
        let mut output = vec![Row::default(); rows];
        let mut arena = vec![0; text];
        let walk = walk(
            &SliceMemory { base: 0, bytes },
            0,
            GS,
            &mut output,
            &mut arena,
        );
        (walk, output, arena)
    }

    #[test]
    fn worker_grows_from_complete_mod_token_count_and_lengths() {
        let mut output = Vec::new();
        let mut text = Vec::new();
        for count in [5_000_u32, 10_000] {
            let mut bytes = fixture();
            let source = bytes.len();
            let mask = count.next_power_of_two() - 1;
            bytes.resize(source + (mask as usize + 2) * STRIDE, 0);
            p64(&mut bytes, MANAGER + TABLE + BUCKETS, source as u64);
            p32(&mut bytes, MANAGER + TABLE + COUNT, count);
            p32(&mut bytes, MANAGER + TABLE + MASK, mask);
            bytes[MANAGER + TABLE + MAXIMUM_PROBE] = 0;
            for ordinal in 0..count {
                let at = source + ordinal as usize * STRIDE;
                bytes[at + DISTANCE] = 1;
                string(
                    &mut bytes,
                    at + KEY,
                    format!("mod_key_{ordinal}").as_bytes(),
                    0,
                );
            }
            bytes[source + (mask as usize + 1) * STRIDE + DISTANCE] = END;
            let result = walk_growing(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &mut output,
                &mut text,
            );
            assert_eq!(
                result.outcome,
                Outcome::Rows {
                    len: count as usize
                }
            );
            let last = output[count as usize - 1];
            assert_eq!(
                &text[last.start..last.start + last.len],
                format!("mod_key_{}", count - 1).as_bytes()
            );
        }
    }

    #[test]
    fn every_key_is_copied_exactly_in_bucket_order() {
        let (walk, rows, arena) = run(&fixture(), 4, 64);
        assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
        assert_eq!(walk.bytes_len, walk.required_bytes);
        let keys: Vec<&[u8]> = rows[..3]
            .iter()
            .map(|row| &arena[row.start..row.start + row.len])
            .collect();
        assert_eq!(keys, [b"b_idea".as_slice(), LONG, b"a_idea"]);
    }

    #[test]
    fn the_constructed_empty_table_has_no_rows() {
        let mut bytes = fixture();
        let table = MANAGER + TABLE;
        p64(&mut bytes, table + BUCKETS, EMPTY_BUCKETS_RVA as u64);
        p32(&mut bytes, table + COUNT, 0);
        p32(&mut bytes, table + MASK, 0);
        bytes[table + MAXIMUM_PROBE] = 0;
        assert_eq!(run(&bytes, 4, 64).0.outcome, Outcome::Rows { len: 0 });
        // A count over the image's buckets is torn.
        p32(&mut bytes, table + COUNT, 1);
        assert!(matches!(
            run(&bytes, 4, 64).0.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn keys_that_do_not_fit_are_counted_with_their_bytes() {
        let (walk, ..) = run(&fixture(), 1, 64);
        assert_eq!(
            walk.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        assert_eq!(walk.required_bytes, 6 + LONG.len() + 6);
        // No storage at all: the live demand.
        let (walk, ..) = run(&fixture(), 0, 0);
        assert_eq!(
            walk.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        assert_eq!(walk.required_bytes, 12 + LONG.len());
    }

    #[test]
    fn a_torn_table_or_unreadable_key_fails() {
        let table = MANAGER + TABLE;
        for (address, value) in [
            // Another class, a count the buckets disagree with, and one
            // beyond them.
            (MANAGER, 1_u32),
            (table + COUNT, 2),
            (table + COUNT, 9),
        ] {
            let mut bytes = fixture();
            p32(&mut bytes, address, value);
            let (walk, ..) = run(&bytes, 4, 64);
            assert!(
                matches!(walk.outcome, Outcome::Unavailable { .. }),
                "{address:#x}: {walk:?}"
            );
        }
        // The end bucket inside the span, and a heap key without its
        // terminator.
        let mut bytes = fixture();
        bytes[BUCKET_ARRAY + 2 * STRIDE + DISTANCE] = END;
        assert!(matches!(
            run(&bytes, 4, 64).0.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut bytes = fixture();
        bytes[HEAP + LONG.len()] = b'!';
        assert!(matches!(
            run(&bytes, 4, 64).0.outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
