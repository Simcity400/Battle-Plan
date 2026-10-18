//! HiredAdvisors held between hours ([`crate::advisor_pool_changes`]).
//!
//! HiredAdvisors, which `has_idea` triggers read (idea membership trigger over the country
//! characters' appointed list), is read every hour. A row records a country's appointed `CAdvisor*`
//! array and each advisor's character id, idea and slot. Every writer of the array is a Hooked site
//! of the advisor pool family  (advisor insertion, the unappointment helper, the pool's constructor
//! and destructor, [`engine_mutations::advisor_pool_mutations`]); every writer of an advisor's character
//! link, idea and slot is one of the advisor family's sites, which notify the character domain
//! ([`crate::character_changes`]); a character's id is written only by `repair_containers`, which
//! the roster domain observes ([`crate::roster_changes`]); and a country gains or replaces its
//! `CCountryCharacters` only through the pool constructor. So while the pool domain's, the
//! character domain's and the roster's quiet tokens and the country and link array headers are
//! those taken before the latest complete walk, every row equals that walk's: such a quiet hour
//! publishes it without reading any country. The quiet hour of the day that
//! [`crate::sampled_verify`] gives the table's key ([`KEY`]; every hour under `appointment_verify`)
//! walks every country instead and compares the walk whole with the held rows; a difference
//! disables the pool domain for the attachment. Any hour that is not quiet walks every country, and
//! an hour holds its walk when the tokens and headers stood still across it.
use crate::{
    advisor_pool_changes::{Counts, Table},
    sampled_verify::Check,
};
use engine_allocation as allocation;
use engine_native::{
    Memory, ccharacter::NameWalk, ccountry_characters as native, ccountry_characters::Row,
    cpdx_array::Array, walk::Outcome,
};

/// Whether HiredAdvisors carries a quiet hour.
pub const CARRY: bool = cfg!(feature = "appointment_changes");
/// Whether every hour walks and compares what a quiet hour would carry.
pub const VERIFY: bool = cfg!(feature = "appointment_verify");
/// The table's stable sampling key: its quiet hour of the day walks and
/// compares.
pub const KEY: u64 = 0;

/// The domains a quiet hour rests on; tests substitute their own.
pub trait Domains: Sync {
    /// The advisor pool domain's token ([`crate::advisor_pool_changes::quiet`]).
    fn pools(&self) -> Option<(u64, u64)>;
    /// The character domain's token ([`crate::character_changes::quiet`]).
    fn characters(&self) -> Option<(u64, u64)>;
    /// The character roster's token ([`crate::roster_changes::roster`]).
    fn roster(&self) -> Option<(u64, u64)>;
    /// A quiet hour's held rows differed from a fresh walk.
    fn mismatch(&self);
}

/// The hooks' domains.
pub struct Observer;
impl Domains for Observer {
    fn pools(&self) -> Option<(u64, u64)> {
        crate::advisor_pool_changes::quiet()
    }
    fn characters(&self) -> Option<(u64, u64)> {
        crate::character_changes::quiet()
    }
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
    fn mismatch(&self) {
        crate::advisor_pool_changes::mismatch();
    }
}

/// What a quiet hour must find unchanged since the held walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pools: (u64, u64),
    characters: (u64, u64),
    roster: (u64, u64),
    /// The country and link array headers.
    arrays: [Array; 2],
}

/// The tokens, then the array headers; `None` when any is unobserved or
/// unreadable.
pub(super) fn key(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    domains: &dyn Domains,
) -> Option<Key> {
    let (pools, characters, roster) = (domains.pools()?, domains.characters()?, domains.roster()?);
    let arrays = engine_native::ccountry::Reader::new(memory, base, game_state)
        .ok()?
        .arrays();
    Some(Key {
        pools,
        characters,
        roster,
        arrays,
    })
}

/// The latest complete walk held for quiet hours, with its key.
#[derive(Default)]
pub struct Appointed {
    key: Option<Key>,
    rows: Vec<Row>,
    bytes: Vec<u8>,
}

impl Appointed {
    /// Nothing held certifies anything.
    pub fn forget(&mut self) {
        self.key = None;
        self.rows.clear();
        self.bytes.clear();
    }

    pub fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<Row>() + self.bytes.capacity()
    }

    /// Holds `rows` and `bytes` under `key`; storage is advisory, so a walk
    /// that cannot be held is read again next hour.
    fn hold(&mut self, key: Key, rows: &[Row], bytes: &[u8]) {
        self.forget();
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.bytes, bytes.len())
        });
        if stored.is_ok() {
            self.rows.extend_from_slice(rows);
            self.bytes.extend_from_slice(bytes);
            self.key = Some(key);
        }
    }
}

/// A row's idea and slot bytes in `bytes`.
fn texts<'a>(row: &Row, bytes: &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let text =
        |start: usize, len: usize| start.checked_add(len).and_then(|end| bytes.get(start..end));
    Some((
        text(row.idea_start, row.idea_len)?,
        text(row.slot_start, row.slot_len)?,
    ))
}

/// Whether two rows record the same appointment.
fn same(held: &Row, held_bytes: &[u8], fresh: &Row, fresh_bytes: &[u8]) -> bool {
    (held.country, held.ordinal, held.character) == (fresh.country, fresh.ordinal, fresh.character)
        && texts(held, held_bytes).is_some()
        && texts(held, held_bytes) == texts(fresh, fresh_bytes)
}

/// Compares the held rows with `fresh`, a whole walk in walk order. Returns
/// the rows compared and whether any differed.
fn compare(held: &[Row], held_bytes: &[u8], fresh: &[Row], fresh_bytes: &[u8]) -> (u64, bool) {
    let mut held = held.iter();
    let mut compared = 0;
    for row in fresh {
        match held.next() {
            Some(kept) if same(kept, held_bytes, row, fresh_bytes) => compared += 1,
            _ => return (compared + 1, true),
        }
    }
    (compared, held.next().is_some())
}

/// Copies `held` into the front of a capture buffer; false when it cannot
/// grow.
pub(super) fn publish<T: Default + Copy>(held: &[T], values: &mut Vec<T>) -> bool {
    if values.len() < held.len() {
        if allocation::reserve(values, held.len() - values.len()).is_err() {
            return false;
        }
        values.resize(held.len(), T::default());
    }
    values[..held.len()].copy_from_slice(held);
    true
}

/// One hour's HiredAdvisors into the capture's `rows` and `bytes`: a quiet
/// hour outside `check`'s hour for [`KEY`] publishes the held walk; any other
/// hour walks every country, and a quiet one compares the walk whole with the
/// held rows. Records the hour's counts.
#[allow(clippy::too_many_arguments)]
pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    held: &mut Appointed,
    domains: &dyn Domains,
    check: Check,
    rows: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    let before = key(memory, base, game_state, domains);
    let quiet = before.is_some() && before == held.key;
    let mut counts = Counts::default();
    if quiet && !check.covers(KEY) && publish(&held.rows, rows) && publish(&held.bytes, bytes) {
        counts.reused = 1;
        counts.reused_rows = held.rows.len() as u64;
        crate::advisor_pool_changes::record(Table::HiredAdvisors, &counts, None);
        return NameWalk {
            outcome: Outcome::Rows {
                len: held.rows.len(),
            },
            bytes_len: held.bytes.len(),
            required_bytes: held.bytes.len(),
        };
    }
    let walk = native::walk_growing(memory, base, game_state, rows, bytes);
    counts.read = 1;
    counts.read_rows = walk.outcome.len() as u64;
    let complete = match walk.outcome {
        Outcome::Rows { len } => rows.get(..len).zip(bytes.get(..walk.bytes_len)),
        _ => None,
    };
    if let (true, Some((walked, walked_bytes))) = (quiet, complete) {
        // What the quiet hour would have published, compared whole.
        let (verified, differ) = compare(&held.rows, &held.bytes, walked, walked_bytes);
        counts.verified = verified;
        if differ {
            counts.mismatches = 1;
            domains.mismatch();
        }
    }
    // Held only when the tokens and headers stood still across the walk.
    match (complete, before) {
        (Some((walked, walked_bytes)), Some(before))
            if key(memory, base, game_state, domains) == Some(before) =>
        {
            held.hold(before, walked, walked_bytes)
        }
        _ => held.forget(),
    }
    crate::advisor_pool_changes::record(Table::HiredAdvisors, &counts, None);
    walk
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(country: u32, ordinal: u32, character: u32, idea: usize) -> Row {
        Row {
            country,
            ordinal,
            character,
            idea_start: idea,
            idea_len: 4,
            slot_start: 0,
            slot_len: 0,
        }
    }

    /// A walk compares with the held rows in order, with their idea and slot
    /// bytes; a changed idea, a missing appointment and an extra one differ.
    #[test]
    fn a_walk_compares_with_the_held_rows() {
        let held_bytes = b"aaaabbbbcccc";
        let held = [row(1, 0, 10, 0), row(2, 0, 20, 4), row(2, 1, 21, 8)];
        let fresh = [row(1, 0, 10, 8), row(2, 0, 20, 4), row(2, 1, 21, 0)];
        assert_eq!(
            compare(&held, held_bytes, &fresh, b"ccccbbbbaaaa"),
            (3, false)
        );
        assert!(compare(&held, held_bytes, &fresh, b"ccccbbbbdddd").1);
        assert!(compare(&held, held_bytes, &held[..2], held_bytes).1);
        let extra = [
            row(1, 0, 10, 0),
            row(2, 0, 20, 4),
            row(2, 1, 21, 8),
            row(4, 0, 40, 0),
        ];
        assert!(compare(&held, held_bytes, &extra, held_bytes).1);
        assert_eq!(compare(&[], b"", &[], b""), (0, false));
    }

    /// The domains of one test: a pool token it moves, and the mismatches
    /// it saw.
    struct Fake {
        pools: std::sync::atomic::AtomicU64,
        mismatches: std::sync::atomic::AtomicU64,
    }
    impl Domains for Fake {
        fn pools(&self) -> Option<(u64, u64)> {
            Some((1, self.pools.load(std::sync::atomic::Ordering::Relaxed)))
        }
        fn characters(&self) -> Option<(u64, u64)> {
            Some((1, 0))
        }
        fn roster(&self) -> Option<(u64, u64)> {
            Some((1, 0))
        }
        fn mismatch(&self) {
            self.mismatches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Without a readable country census nothing is quiet and nothing is
    /// held: every hour walks (and here fails, unreadable).
    #[test]
    fn an_unreadable_census_holds_nothing_and_walks_every_hour() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let domains = Fake {
            pools: 0.into(),
            mismatches: 0.into(),
        };
        let mut held = Appointed::default();
        let (mut rows, mut bytes) = (Vec::new(), Vec::new());
        for hour in 0..3 {
            let walk = capture(
                &memory,
                0,
                0x1000,
                &mut held,
                &domains,
                Check::Slice { hour },
                &mut rows,
                &mut bytes,
            );
            assert!(matches!(walk.outcome, Outcome::Unavailable { .. }));
            assert!(held.key.is_none());
        }
        assert_eq!(
            domains
                .mismatches
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// A game state whose country table holds one null slot and whose link
    /// table holds its sentinel: the census reads, and a walk finds no
    /// appointment.
    fn empty_world() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x2000];
        let mut header = |at: usize, data: usize, count: u32| {
            bytes[at..at + 8].copy_from_slice(&(0x1000 + data as u64).to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        };
        header(engine_native::cgame_state::COUNTRIES, 0x1000, 2);
        header(engine_native::cgame_state::COUNTRY_LINKS, 0x1100, 1);
        bytes
    }

    /// A quiet hour publishes the held walk without walking; the quiet hour
    /// of the table's rotation walks and compares, and a held row the walk
    /// does not find is a mismatch; a moved token walks without comparing.
    #[test]
    fn a_quiet_hour_publishes_the_held_walk_and_its_rotation_hour_compares() {
        let world = empty_world();
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &world,
        };
        let domains = Fake {
            pools: 0.into(),
            mismatches: 0.into(),
        };
        let mut held = Appointed::default();
        let (mut rows, mut bytes) = (Vec::new(), Vec::new());
        let mut hour = |held: &mut Appointed, hour: i32| {
            capture(
                &memory,
                0,
                0x1000,
                held,
                &domains,
                Check::Slice { hour },
                &mut rows,
                &mut bytes,
            )
            .outcome
        };
        // The first hour walks and holds the (empty) walk.
        assert_eq!(hour(&mut held, 1), Outcome::Rows { len: 0 });
        assert!(held.key.is_some());
        // A held appointment stands in for the walk's: a quiet hour publishes
        // it, so this hour did not walk.
        held.rows.push(row(7, 0, 70, 0));
        held.bytes.extend_from_slice(b"idea");
        assert_eq!(hour(&mut held, 2), Outcome::Rows { len: 1 });
        // The rotation hour of key 0 walks, finds no appointment and reports
        // the held one as a missed write.
        assert_eq!(hour(&mut held, 24), Outcome::Rows { len: 0 });
        let mismatches = || {
            domains
                .mismatches
                .load(std::sync::atomic::Ordering::Relaxed)
        };
        assert_eq!(mismatches(), 1);
        // A writer moved the pool token: the hour walks and compares nothing.
        held.rows.push(row(7, 0, 70, 0));
        held.bytes.extend_from_slice(b"idea");
        domains
            .pools
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(hour(&mut held, 3), Outcome::Rows { len: 0 });
        assert_eq!(mismatches(), 1);
        assert!(held.rows.is_empty() && held.key.is_some());
    }
}
