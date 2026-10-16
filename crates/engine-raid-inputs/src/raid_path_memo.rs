//! Raid-system path-distance memo shared by land and naval lookup.
//! The key contains source and target province IDs only, so the first computed
//! path kind wins for that pair during the session, including failures. The memo
//! is not saved. Buckets retain hash, occupancy/probe distance, province IDs and
//! distance/success fields.
//!
//! Game writers hold a spin lock across lookup, insertion and rehash. Capture
//! does not take that lock: it snapshots header and lock before and after copying,
//! accepting only unchanged headers with a free lock at both ends. Insertions change
//! element count and rehash changes geometry, so overlapping work makes the copy
//! torn and this hour's table unavailable. A failed check is counted without retry.

use crate::{Memory, cgame_state, craid_system, memory::read_slice, read, walk::Outcome};

/// `CRaidSystem + 0x230`: the path distance memo.
pub const MEMO: usize = 0x230;
const BUCKETS: usize = 0x08;
const COUNT: usize = 0x10;
const MASK: usize = 0x14;
const MAX_PROBE: usize = 0x18;
/// Spin lock acquired by compare-exchanging zero to one.
const LOCK: usize = 0x20;
/// The header span a read snapshots: buckets through the lock word.
const HEADER: usize = LOCK + 4 - BUCKETS;
const BUCKET: usize = 0x20;
const PROBE: usize = 0x04;
const SOURCE: usize = 0x08;
const TARGET: usize = 0x0c;
const DISTANCE: usize = 0x10;
const OK: usize = 0x18;
const END: u8 = 0xff;
const BATCH: usize = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source: u32,
    pub target: u32,
    /// Fixed5 map units; meaningful only when `ok`.
    pub distance: i64,
    pub ok: u8,
}

/// Header-only sizing hint; no value here is recorded. Rehash allocates the
/// bucket and probe-tail extent plus a separate sentinel. Inserts keep that
/// geometry, bounding output even if count grows before the next sizing poll.
pub fn required_rows(memory: &impl Memory, base: usize, game_state: usize) -> Option<usize> {
    let memo = memo_address(memory, base, game_state).ok()?;
    table_slots(&read_header(memory, memo).ok()?).ok()
}

/// Every memo entry, ordered by (source, target). Capacity shortfalls
/// truncate and count the unread entries. `torn` counts copies rejected
/// because a writer overlapped them (see the module documentation).
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [Row],
    torn: &mut u32,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(rows),
        torn,
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    torn: &mut u32,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(rows),
        torn,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: crate::walk::Buffer<'_, Row>,
    torn: &mut u32,
) -> Outcome {
    match walk_rows(memory, base, game_state, rows, torn) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_rows(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: crate::walk::Buffer<'_, Row>,
    torn: &mut u32,
) -> Result<Outcome, &'static str> {
    let memo = memo_address(memory, base, game_state)?;
    let before = read_header(memory, memo)?;
    let copied = copy_table(memory, &before, rows);
    let after = read_header(memory, memo)?;
    let locked = |header: &[u8; HEADER]| header[LOCK - BUCKETS..] != [0; 4];
    if before != after || locked(&before) || locked(&after) {
        *torn += 1;
        return Err("raid path memo was written during the read");
    }
    // A quiet table that fails validation is malformed, not torn.
    copied
}

fn memo_address(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let system = game_state
        .checked_add(cgame_state::RAID_SYSTEM)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("raid system unreadable or null")?;
    let vtable = base
        .checked_add(craid_system::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if read::<u64>(memory, system) != Some(vtable) {
        return Err("raid system vtable mismatch");
    }
    system
        .checked_add(MEMO)
        .ok_or("raid path memo address overflow")
}

fn read_header(memory: &impl Memory, memo: usize) -> Result<[u8; HEADER], &'static str> {
    let mut bytes = [0_u8; HEADER];
    memo.checked_add(BUCKETS)
        .is_some_and(|at| read_slice(memory, at, &mut bytes))
        .then_some(bytes)
        .ok_or("raid path memo header unreadable")
}

fn table_slots(header: &[u8; HEADER]) -> Result<usize, &'static str> {
    let mask = u32::from_le_bytes(header[MASK - BUCKETS..][..4].try_into().unwrap());
    let slots = mask
        .checked_add(1)
        .filter(|slots| slots.is_power_of_two())
        .ok_or("raid path memo mask is not a power of two less one")?;
    // The probe tail precedes the separate end sentinel.
    (slots as usize)
        .checked_add(usize::from(header[MAX_PROBE - BUCKETS]))
        .ok_or("raid path memo slot count overflow")
}

/// Copy and validate the buckets the header snapshot describes.
fn copy_table(
    memory: &impl Memory,
    header: &[u8; HEADER],
    mut rows: crate::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let field = |at: usize| &header[at - BUCKETS..];
    let buckets = usize::try_from(u64::from_le_bytes(field(BUCKETS)[..8].try_into().unwrap()))
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or("raid path memo buckets unreadable or null")?;
    let count = u32::from_le_bytes(field(COUNT)[..4].try_into().unwrap());
    let max_probe = field(MAX_PROBE)[0];
    rows.ensure(count as usize)?;
    let slots = table_slots(header)?;
    let mut chunk = [0_u8; BATCH * BUCKET];
    let (mut len, mut occupied, mut unread) = (0, 0_u32, 0_u32);
    let mut first = 0;
    while first <= slots {
        let batch = (slots + 1 - first).min(BATCH);
        let address = buckets
            .checked_add(first * BUCKET)
            .ok_or("raid path memo address overflow")?;
        let bytes = &mut chunk[..batch * BUCKET];
        if !read_slice(memory, address, bytes) {
            return Err("raid path memo buckets unreadable");
        }
        for (index, bucket) in bytes.chunks_exact(BUCKET).enumerate() {
            let probe = bucket[PROBE];
            if first + index == slots {
                if probe != END {
                    return Err("raid path memo end sentinel missing");
                }
                continue;
            }
            if probe == 0 {
                continue;
            }
            if probe == END || probe > max_probe {
                return Err("raid path memo probe length invalid");
            }
            occupied += 1;
            if bucket[OK] > 1 {
                return Err("raid path memo ok byte invalid");
            }
            if len == rows.len() {
                unread += 1;
                continue;
            }
            let word = |at: usize| u32::from_le_bytes(bucket[at..at + 4].try_into().unwrap());
            rows[len] = Row {
                source: word(SOURCE),
                target: word(TARGET),
                distance: i64::from_le_bytes(bucket[DISTANCE..DISTANCE + 8].try_into().unwrap()),
                ok: bucket[OK],
            };
            len += 1;
        }
        first += batch;
    }
    if occupied != count {
        return Err("raid path memo count disagrees with its buckets");
    }
    rows[..len].sort_unstable_by_key(|row| (row.source, row.target));
    if rows[..len]
        .windows(2)
        .any(|pair| (pair[0].source, pair[0].target) == (pair[1].source, pair[1].target))
    {
        return Err("raid path memo holds a duplicate key");
    }
    Ok(if unread == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: unread,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const GAME: usize = 0x100;
    const SYSTEM: usize = 0x800;
    const TABLE: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    type Bucket = (usize, u8, u32, u32, i64, u8);

    /// Module base zero, so the vtable is its RVA.
    fn fixture(buckets: &[Bucket], mask: u32, probe: u8, count: u32) -> Vec<u8> {
        let mut bytes = vec![0; 0x2000];
        put(
            &mut bytes,
            GAME + cgame_state::RAID_SYSTEM,
            &(SYSTEM as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM,
            &(craid_system::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM + MEMO + BUCKETS,
            &(TABLE as u64).to_le_bytes(),
        );
        put(&mut bytes, SYSTEM + MEMO + COUNT, &count.to_le_bytes());
        put(&mut bytes, SYSTEM + MEMO + MASK, &mask.to_le_bytes());
        put(&mut bytes, SYSTEM + MEMO + MAX_PROBE, &[probe]);
        let slots = mask as usize + usize::from(probe) + 1;
        put(&mut bytes, TABLE + slots * BUCKET + PROBE, &[END]);
        for &(slot, probe, source, target, distance, ok) in buckets {
            let at = TABLE + slot * BUCKET;
            put(&mut bytes, at + PROBE, &[probe]);
            put(&mut bytes, at + SOURCE, &source.to_le_bytes());
            put(&mut bytes, at + TARGET, &target.to_le_bytes());
            put(&mut bytes, at + DISTANCE, &distance.to_le_bytes());
            put(&mut bytes, at + OK, &[ok]);
        }
        bytes
    }

    fn run(bytes: &[u8], rows: &mut [Row]) -> Outcome {
        let mut torn = 0;
        let outcome = walk(&SliceMemory { base: 0, bytes }, 0, GAME, rows, &mut torn);
        assert_eq!(torn, 0, "a quiet table is never torn");
        outcome
    }

    #[test]
    fn required_rows_reads_slot_geometry_without_buckets() {
        let size = |bytes: &[u8]| required_rows(&SliceMemory { base: 0, bytes }, 0, GAME);
        let cases = [(0, 0, 0, 1), (7, 1, 0, 9), (7, 4, 1, 12), (7, 4, 4, 12)];
        for (mask, probe, count, required) in cases {
            let bytes = fixture(&[], mask, probe, count);
            // The buckets are deliberately outside readable memory.
            assert_eq!(size(&bytes[..TABLE]), Some(required));
        }
        let mut bytes = fixture(&[], 7, 4, 0);
        for mask in [2, u32::MAX] {
            put(&mut bytes, SYSTEM + MEMO + MASK, &mask.to_le_bytes());
            assert_eq!(size(&bytes), None);
        }
        let bytes = fixture(&[], 7, 4, 0);
        assert_eq!(size(&bytes[..SYSTEM + MEMO + BUCKETS + HEADER - 1]), None);
        let mut bytes = bytes;
        put(&mut bytes, SYSTEM, &0_u64.to_le_bytes());
        assert_eq!(size(&bytes), None);
    }

    #[test]
    fn required_rows_holds_later_inserts_in_the_probe_tail() {
        let empty = fixture(&[], 7, 4, 0);
        let required = required_rows(
            &SliceMemory {
                base: 0,
                bytes: &empty,
            },
            0,
            GAME,
        )
        .unwrap();
        // Inserts grow COUNT without changing the mask or the probe limit.
        let filled = fixture(
            &[(7, 1, 1, 2, 3, 1), (8, 2, 2, 3, 4, 0), (10, 4, 3, 4, 5, 1)],
            7,
            4,
            3,
        );
        let mut rows = vec![Row::default(); required];
        assert_eq!(run(&filled, &mut rows), Outcome::Rows { len: 3 });
    }

    #[test]
    fn a_held_lock_or_a_moving_header_is_a_counted_torn_read() {
        let buckets = [(3, 1, 7, 9, 500_000, 1)];
        let mut rows = [Row::default(); 4];
        // Held by a writer: this hour's table is not published.
        let mut locked = fixture(&buckets, 7, 1, 1);
        put(&mut locked, SYSTEM + MEMO + LOCK, &1_u32.to_le_bytes());
        let mut torn = 0;
        let outcome = walk(
            &SliceMemory {
                base: 0,
                bytes: &locked,
            },
            0,
            GAME,
            &mut rows,
            &mut torn,
        );
        assert!(matches!(outcome, Outcome::Unavailable { .. }));
        assert_eq!(torn, 1);
        // An insert lands during the copy: counted and unavailable, not
        // read again.
        let quiet = fixture(&buckets, 7, 1, 1);
        let memory = Inserting {
            bytes: std::cell::RefCell::new(quiet),
            reads: std::cell::Cell::new(0),
        };
        let mut torn = 0;
        assert_eq!(
            walk(&memory, 0, GAME, &mut rows, &mut torn),
            Outcome::Unavailable {
                reason: "raid path memo was written during the read"
            }
        );
        assert_eq!(torn, 1);
    }

    /// Bumps the element count after the first header read, as a writer that
    /// finishes an insert while the buckets are copied.
    struct Inserting {
        bytes: std::cell::RefCell<Vec<u8>>,
        reads: std::cell::Cell<u32>,
    }

    impl Memory for Inserting {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let bytes = self.bytes.borrow().clone();
            let copied = SliceMemory {
                base: 0,
                bytes: &bytes,
            }
            .copy(address, destination);
            if address == SYSTEM + MEMO + BUCKETS {
                let reads = self.reads.get() + 1;
                self.reads.set(reads);
                if reads == 1 {
                    // The first snapshot saw one entry; the writer then adds
                    // a second and the count moves before the second snapshot.
                    let mut bytes = self.bytes.borrow_mut();
                    put(&mut bytes, TABLE + 5 * BUCKET + PROBE, &[1]);
                    put(
                        &mut bytes,
                        TABLE + 5 * BUCKET + SOURCE,
                        &8_u32.to_le_bytes(),
                    );
                    put(
                        &mut bytes,
                        TABLE + 5 * BUCKET + TARGET,
                        &9_u32.to_le_bytes(),
                    );
                    put(&mut bytes, SYSTEM + MEMO + COUNT, &2_u32.to_le_bytes());
                }
            }
            copied
        }
    }

    #[test]
    fn entries_come_back_by_key_with_failures_kept() {
        let bytes = fixture(&[(3, 1, 9, 2, 700_000, 1), (1, 2, 4, 5, 0, 0)], 3, 2, 2);
        let mut rows = [Row::default(); 4];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[..2],
            [
                Row {
                    source: 4,
                    target: 5,
                    distance: 0,
                    ok: 0
                },
                Row {
                    source: 9,
                    target: 2,
                    distance: 700_000,
                    ok: 1
                },
            ]
        );
        let mut one = [Row::default(); 1];
        assert_eq!(
            run(&bytes, &mut one),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn a_count_sentinel_or_probe_mismatch_is_refused() {
        let mut rows = [Row::default(); 4];
        let wrong_count = fixture(&[(0, 1, 1, 2, 3, 1)], 1, 1, 2);
        assert!(matches!(
            run(&wrong_count, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let mut no_end = fixture(&[], 1, 1, 0);
        put(&mut no_end, TABLE + 3 * BUCKET + PROBE, &[0]);
        assert!(matches!(
            run(&no_end, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let too_far = fixture(&[(0, 3, 1, 2, 3, 1)], 1, 1, 1);
        assert!(matches!(
            run(&too_far, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let empty = fixture(&[], 0, 0, 0);
        assert_eq!(run(&empty, &mut rows), Outcome::Rows { len: 0 });
    }

    /// `entries` pairs in a table at most seven-eighths full, one per slot.
    fn scaled(entries: usize) -> Vec<u8> {
        let slots = (entries * 8 / 7).next_power_of_two().max(2);
        let table = 0x1000;
        let mut bytes = vec![0; table + (slots + 2) * BUCKET];
        put(
            &mut bytes,
            GAME + cgame_state::RAID_SYSTEM,
            &(SYSTEM as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM,
            &(craid_system::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM + MEMO + BUCKETS,
            &(table as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM + MEMO + COUNT,
            &(entries as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM + MEMO + MASK,
            &((slots - 1) as u32).to_le_bytes(),
        );
        put(&mut bytes, SYSTEM + MEMO + MAX_PROBE, &[1]);
        put(&mut bytes, table + (slots + 1) * BUCKET + PROBE, &[END]);
        for entry in 0..entries {
            let at = table + entry * BUCKET;
            put(&mut bytes, at + PROBE, &[1]);
            put(&mut bytes, at + SOURCE, &(entry as u32).to_le_bytes());
            put(&mut bytes, at + TARGET, &7_u32.to_le_bytes());
            put(&mut bytes, at + DISTANCE, &100_000_i64.to_le_bytes());
            put(&mut bytes, at + OK, &[1]);
        }
        bytes
    }

    #[test]
    fn required_rows_covers_memos_past_the_default_capacity() {
        for entries in [1_149, 1_500, 5_000] {
            let bytes = scaled(entries);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            if entries == 1_149 {
                let mut old = vec![Row::default(); 1_024];
                assert_eq!(
                    run(&bytes, &mut old),
                    Outcome::Truncated {
                        len: 1_024,
                        unread_rows: 125
                    }
                );
            }
            let mut rows = vec![Row::default(); required_rows(&memory, 0, GAME).unwrap()];
            assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: entries });
        }
    }

    #[test]
    #[ignore]
    fn bench_path_memo_by_entries() {
        for entries in [1_000, 10_000, 50_000] {
            let bytes = scaled(entries);
            let memory = crate::memory::CountingMemory::new(&bytes);
            let mut rows = vec![Row::default(); entries];
            memory.bench(
                &format!("RaidPathDistances {entries} entries"),
                20,
                |memory| {
                    let outcome = walk(memory, 0, GAME, &mut rows, &mut 0);
                    assert_eq!(outcome, Outcome::Rows { len: entries });
                    format!("{entries} rows")
                },
            );
        }
    }
}
