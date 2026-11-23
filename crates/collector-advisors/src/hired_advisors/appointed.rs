use crate::advisor_pool_changes::{Counts, Table};
use engine_allocation as allocation;
use engine_native::{Memory, ccharacter::NameWalk, cpdx_array::Array, walk::Outcome};
use engine_native_politics::ccountry_characters::{self as native, Row};

pub const CARRY: bool = cfg!(feature = "appointment_changes");

pub trait Domains: Sync {
    fn pools(&self) -> Option<(u64, u64)>;
    fn characters(&self) -> Option<(u64, u64)>;
    fn roster(&self) -> Option<(u64, u64)>;
}

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pools: (u64, u64),
    characters: (u64, u64),
    roster: (u64, u64),
    arrays: [Array; 2],
}

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

#[derive(Default)]
pub struct Appointed {
    key: Option<Key>,
    rows: Vec<Row>,
    bytes: Vec<u8>,
    generation: u64,
    published: Option<u64>,
}

impl Appointed {
    pub fn forget(&mut self) {
        self.key = None;
        self.rows.clear();
        self.bytes.clear();
        self.published = None;
    }

    pub fn published(&self) -> Option<u64> {
        self.published
    }

    pub fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<Row>() + self.bytes.capacity()
    }

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
            self.generation = collector_projection::projection::generation();
            self.published = Some(self.generation);
        }
    }
}

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

pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    held: &mut Appointed,
    domains: &dyn Domains,
    rows: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    let before = key(memory, base, game_state, domains);
    let quiet = before.is_some() && before == held.key;
    let mut counts = Counts::default();
    held.published = None;
    if quiet && publish(&held.rows, rows) && publish(&held.bytes, bytes) {
        held.published = Some(held.generation);
        counts.reused = 1;
        counts.reused_rows = held.rows.len() as u64;
        // SAFETY: Called from admitted hired-advisor capture with no pool-demand override.
        unsafe { crate::advisor_pool_changes::record(Table::HiredAdvisors, &counts, None) };
        return NameWalk {
            outcome: Outcome::Rows {
                len: held.rows.len(),
            },
            bytes_len: held.bytes.len(),
            required_bytes: held.bytes.len(),
        };
    }
    let walk = native::walk(memory, base, game_state, rows, bytes);
    counts.read = 1;
    counts.read_rows = walk.outcome.len() as u64;
    let complete = match walk.outcome {
        Outcome::Rows { len } => rows.get(..len).zip(bytes.get(..walk.bytes_len)),
        _ => None,
    };
    match (complete, before) {
        (Some((walked, walked_bytes)), Some(before))
            if key(memory, base, game_state, domains) == Some(before) =>
        {
            held.hold(before, walked, walked_bytes)
        }
        _ => held.forget(),
    }
    // SAFETY: Called from admitted hired-advisor capture with no pool-demand override.
    unsafe { crate::advisor_pool_changes::record(Table::HiredAdvisors, &counts, None) };
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

    struct Fake {
        pools: std::sync::atomic::AtomicU64,
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
    }

    #[test]
    fn an_unreadable_census_holds_nothing_and_walks_every_hour() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let domains = Fake { pools: 0.into() };
        let mut held = Appointed::default();
        let (mut rows, mut bytes) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            let walk = capture(
                &memory, 0, 0x1000, &mut held, &domains, &mut rows, &mut bytes,
            );
            assert!(matches!(walk.outcome, Outcome::Unavailable { .. }));
            assert!(held.key.is_none() && held.published().is_none());
        }
    }

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

    #[test]
    fn a_quiet_hour_publishes_the_held_walk_and_a_moved_token_walks() {
        let world = empty_world();
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &world,
        };
        let domains = Fake { pools: 0.into() };
        let mut held = Appointed::default();
        let (mut rows, mut bytes) = (Vec::new(), Vec::new());
        let mut hour = |held: &mut Appointed| {
            capture(&memory, 0, 0x1000, held, &domains, &mut rows, &mut bytes).outcome
        };
        assert_eq!(hour(&mut held), Outcome::Rows { len: 0 });
        assert!(held.key.is_some());
        let first = held.published().unwrap();
        held.rows.push(row(7, 0, 70, 0));
        held.bytes.extend_from_slice(b"idea");
        assert_eq!(hour(&mut held), Outcome::Rows { len: 1 });
        assert_eq!(hour(&mut held), Outcome::Rows { len: 1 });
        assert_eq!(held.published(), Some(first));
        domains
            .pools
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(hour(&mut held), Outcome::Rows { len: 0 });
        assert!(held.rows.is_empty() && held.key.is_some());
        assert!(
            held.published()
                .is_some_and(|generation| generation != first)
        );
    }
}
