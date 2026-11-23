use super::appointed::{Domains, Key, key, publish};
use crate::advisor_pool_changes::{Counts, Table};
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use engine_native_politics::ccountry_characters as native;

pub const CARRY: bool = cfg!(feature = "character_record_changes");

#[derive(Default)]
pub struct Records {
    key: Option<Key>,
    active: Vec<native::ActiveRow>,
    retired: Vec<native::ActiveRow>,
    generation: u64,
    published: Option<u64>,
}

impl Records {
    pub fn forget(&mut self) {
        self.key = None;
        self.active.clear();
        self.retired.clear();
        self.published = None;
    }

    pub fn published(&self) -> Option<u64> {
        self.published
    }

    pub fn measure(&self) -> usize {
        (self.active.capacity() + self.retired.capacity()) * size_of::<native::ActiveRow>()
    }

    fn hold(&mut self, key: Key, active: &[native::ActiveRow], retired: &[native::ActiveRow]) {
        self.forget();
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.active, active.len())?;
            allocation::reserve(&mut self.retired, retired.len())
        });
        if stored.is_ok() {
            self.active.extend_from_slice(active);
            self.retired.extend_from_slice(retired);
            self.key = Some(key);
            self.generation = collector_projection::projection::generation();
            self.published = Some(self.generation);
        }
    }
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    held: &mut Records,
    domains: &dyn Domains,
    active: &mut Vec<native::ActiveRow>,
    retired: &mut Vec<native::ActiveRow>,
) -> [Outcome; 2] {
    let before = key(memory, base, game_state, domains);
    let quiet = before.is_some() && before == held.key;
    let mut counts = Counts::default();
    let rows = |active: usize, retired: usize| (active + retired) as u64;
    held.published = None;
    if quiet && publish(&held.active, active) && publish(&held.retired, retired) {
        held.published = Some(held.generation);
        counts.reused = 1;
        counts.reused_rows = rows(held.active.len(), held.retired.len());
        // SAFETY: Called from admitted character-records capture with no pool-demand override.
        unsafe { crate::advisor_pool_changes::record(Table::CharacterRecords, &counts, None) };
        return [&held.active, &held.retired].map(|held| Outcome::Rows { len: held.len() });
    }
    let outcomes = native::walk_records(memory, base, game_state, active, retired);
    counts.read = 1;
    counts.read_rows = rows(outcomes[0].len(), outcomes[1].len());
    let complete = match outcomes {
        [
            Outcome::Rows { len: active_len },
            Outcome::Rows { len: retired_len },
        ] => active.get(..active_len).zip(retired.get(..retired_len)),
        _ => None,
    };
    match (complete, before) {
        (Some((walked, retired)), Some(before))
            if key(memory, base, game_state, domains) == Some(before) =>
        {
            held.hold(before, walked, retired)
        }
        _ => held.forget(),
    }
    // SAFETY: Called from admitted character-records capture with no pool-demand override.
    unsafe { crate::advisor_pool_changes::record(Table::CharacterRecords, &counts, None) };
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    #[derive(Default)]
    struct Fake {
        pools: AtomicU64,
    }
    impl Domains for Fake {
        fn pools(&self) -> Option<(u64, u64)> {
            Some((1, self.pools.load(Relaxed)))
        }
        fn characters(&self) -> Option<(u64, u64)> {
            Some((1, 0))
        }
        fn roster(&self) -> Option<(u64, u64)> {
            Some((1, 0))
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

    fn record(country: u32, character: u32) -> native::ActiveRow {
        native::ActiveRow {
            country,
            ordinal: 0,
            character: Some(character),
        }
    }

    #[test]
    fn a_quiet_hour_publishes_the_held_records_and_a_moved_token_walks() {
        let world = empty_world();
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &world,
        };
        let domains = Fake::default();
        let mut held = Records::default();
        let (mut active, mut retired) = (Vec::new(), Vec::new());
        let mut hour = |held: &mut Records| {
            capture(
                &memory,
                0,
                0x1000,
                held,
                &domains,
                &mut active,
                &mut retired,
            )
        };
        let rows = |active: usize, retired: usize| {
            [
                Outcome::Rows { len: active },
                Outcome::Rows { len: retired },
            ]
        };
        assert_eq!(hour(&mut held), rows(0, 0));
        assert!(held.key.is_some());
        let first = held.published().unwrap();
        held.active.push(record(7, 70));
        held.retired.push(record(7, 71));
        assert_eq!(hour(&mut held), rows(1, 1));
        assert_eq!(hour(&mut held), rows(1, 1));
        assert_eq!(held.published(), Some(first));
        domains.pools.fetch_add(1, Relaxed);
        assert_eq!(hour(&mut held), rows(0, 0));
        assert!(held.active.is_empty() && held.retired.is_empty() && held.key.is_some());
        assert!(
            held.published()
                .is_some_and(|generation| generation != first)
        );
    }
}
