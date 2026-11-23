use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

type Key = (usize, u32, u32);
static GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
pub(super) struct Store {
    scope: (usize, usize),
    daily: bool,
    pub(super) generation: u64,
    entries: Vec<(Key, i64, bool, u32)>,
    sorted: usize,
    charge: Option<collector_primitives::raw_memory::Charge>,
}

pub(super) static STORE: Mutex<Store> = Mutex::new(Store {
    scope: (0, 0),
    daily: false,
    generation: 0,
    entries: Vec::new(),
    sorted: 0,
    charge: None,
});

impl Store {
    pub(super) fn forget(&mut self) {
        self.scope = (0, 0);
        self.daily = false;
        self.generation = 0;
        self.entries.clear();
        self.sorted = 0;
    }

    pub(super) fn begin(&mut self, scope: (usize, usize), full: bool, daily: bool) {
        if full || self.scope != scope || self.generation == 0 || daily && !self.daily {
            self.entries.clear();
            self.sorted = 0;
            self.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
            self.scope = scope;
        }
        self.daily = daily;
        for entry in &mut self.entries {
            entry.2 = false;
        }
    }

    pub(super) fn get(&mut self, key: Key) -> Option<i64> {
        if self.daily {
            return None;
        }
        let at = match self.entries[..self.sorted].binary_search_by_key(&key, |entry| entry.0) {
            Ok(at) => at,
            Err(_) => {
                self.sorted
                    + self.entries[self.sorted..]
                        .iter()
                        .rposition(|entry| entry.0 == key)?
            }
        };
        self.entries[at].2 = true;
        Some(self.entries[at].1)
    }

    pub(super) fn put(&mut self, key: Key, value: i64) {
        if let Ok(at) = self.entries[..self.sorted].binary_search_by_key(&key, |entry| entry.0) {
            (self.entries[at].1, self.entries[at].2) = (value, true);
            return;
        }
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.entries, 1)).is_err() {
            if let Some(entry) = self.entries[self.sorted..].iter_mut().rfind(|entry| entry.0 == key) {
                entry.1 = value;
            }
            return;
        }
        let in_order = self.sorted == self.entries.len()
            && self.entries.last().is_none_or(|last| last.0 < key);
        let position = self.entries.len() as u32;
        self.entries.push((key, value, true, position));
        if in_order {
            self.sorted += 1;
        }
    }

    fn settle(&mut self) {
        if self.sorted == self.entries.len() {
            return;
        }
        self.entries.sort_unstable_by_key(|entry| (entry.0, entry.3));
        self.entries.dedup_by(|later, earlier| {
            let same = later.0 == earlier.0;
            if same {
                *earlier = *later;
            }
            same
        });
        self.sorted = self.entries.len();
    }

    pub(super) fn observe_prepared(&mut self, log: &collector_raw::raw::RawLog<{ super::N }>) {
        self.settle();
        for span in log
            .spans
            .iter()
            .filter(|span| span.ok && span.len == 8 && span.mask & (1 << 3) != 0)
        {
            let Some(pointer) = span
                .addr
                .checked_sub(engine_native::cunit_leader::EXPERIENCE)
            else {
                continue;
            };
            let at = self.entries.partition_point(|entry| entry.0.0 < pointer);
            for entry in self.entries[at..]
                .iter_mut()
                .take_while(|entry| entry.0.0 == pointer)
            {
                let start = span.start as usize;
                entry.1 = i64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap());
                entry.2 = true;
            }
        }
    }

    pub(super) fn touch(&mut self, pointer: usize, character: u32) {
        self.settle();
        let at = self.entries.partition_point(|entry| entry.0.0 < pointer);
        for entry in self.entries[at..]
            .iter_mut()
            .take_while(|entry| entry.0.0 == pointer)
        {
            if entry.0.1 == character {
                entry.2 = true;
            }
        }
    }

    pub(super) fn finish(&mut self, complete: bool) {
        if complete {
            self.settle();
            self.entries.retain(|entry| entry.2);
        } else {
            self.entries.clear();
        }
        self.sorted = self.entries.len();
        if !complete {
            self.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        self.charge.get_or_insert_with(Default::default).set(
            collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(&self.entries),
                ..Default::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extended::characters::{
        census_tests::{MODULE, World, capacities, context},
        *,
    };

    struct DailyReadGuard<'a> {
        world: &'a World,
        held_xp: &'a [usize],
    }
    impl Memory for DailyReadGuard<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let end = address.saturating_add(output.len());
            let overlaps = |start: usize, len| address < start + len && start < end;
            for &pointer in self.held_xp {
                assert!(
                    !overlaps(pointer + cunit_leader::EXPERIENCE, 8),
                    "held XP read natively"
                );
            }
            for index in 0..self.world.characters {
                if let Some(pointer) = World::unit(index) {
                    for field in [
                        cunit_leader::TRAIT_EXPERIENCE_DATA,
                        cunit_leader::TRAIT_EXPERIENCE_FACTORS,
                    ] {
                        assert!(
                            !overlaps(pointer + field, 16),
                            "off-slot training vector read"
                        );
                    }
                }
            }
            self.world.copy(address, output)
        }
    }

    fn acquire(memory: &impl Memory, job: &mut CharactersJob, daily: bool, full: bool) -> Captured {
        let mut active = PART_MASKS[0];
        active[5] = daily || full;
        let mut ctx = context(active);
        ctx.force_full = full;
        let report = copy_characters(memory, ctx, job, Roles::Gather);
        assert!(report.raw_ok && !report.plan_failed);
        let mut frame = Captured::with_capacities(&capacities());
        frame.set_character_activity(active);
        frame.decode_characters_payload(MODULE, job, &mut [0; N]);
        frame.remember_templates(job);
        job.prepare_read_plans();
        job.clear();
        frame
    }

    fn xp(frame: &Captured, id: u32) -> i64 {
        let TableView::Complete(TableRows::UnitLeaders(rows)) = frame.tables()[3].clone() else {
            panic!("complete leaders: {:?}", frame.outcomes());
        };
        rows.iter().find(|row| row.id == id).unwrap().experience
    }

    #[test]
    fn daily_xp_skips_native_words_across_jobs_and_reads_births_reuse_and_refresh() {
        let _globals = crate::extended::characters::test_globals();
        let mut world = World::new(8);
        let army = World::unit(5).unwrap();
        let mut first = CharactersJob::new_part(0);
        let before = acquire(&world, &mut first, true, true);
        let held = xp(&before, 2_000_005);
        assert!(matches!(before.tables()[5], TableView::Complete(_)));
        world.patch(
            army + cunit_leader::EXPERIENCE,
            &(-765_432_i64).to_le_bytes(),
        );
        world.patch(army + carmy_leader::ATTACK, &77_i32.to_le_bytes());
        world.characters = 12;
        world.historical = 11;
        let navy = World::unit(9).unwrap();
        world.patch(navy + cunit_leader::EXPERIENCE, &(-123_i64).to_le_bytes());
        let guard = DailyReadGuard {
            world: &world,
            held_xp: &[army],
        };
        let mut next = CharactersJob::new_part(0);
        let after = acquire(&guard, &mut next, false, false);
        assert_eq!(xp(&after, 2_000_005), held);
        assert_eq!(xp(&after, 2_000_009), -123, "new roles read immediately");
        assert!(
            matches!(after.tables()[5], TableView::Unavailable { ref reason }
            if reason == replay_data::cadence::SKIPPED_REASON)
        );
        assert_eq!(
            after
                .units
                .rows
                .rows
                .iter()
                .find(|row| row.native.id == 2_000_005)
                .unwrap()
                .native
                .components[0],
            77
        );
        world.patch(army + cunit_leader::ID, &99_u32.to_le_bytes());
        let guard = DailyReadGuard {
            world: &world,
            held_xp: &[navy],
        };
        let reused = acquire(&guard, &mut first, false, false);
        assert_eq!(xp(&reused, 99), -765_432);
        world.patch(army + cunit_leader::EXPERIENCE, &i64::MIN.to_le_bytes());
        let refreshed = acquire(&world, &mut next, false, true);
        assert_eq!(xp(&refreshed, 99), i64::MIN);
        assert!(matches!(refreshed.tables()[5], TableView::Complete(_)));
    }

    #[test]
    fn daily_xp_repeated_unwind_cannot_restore_an_old_shared_sample() {
        let _globals = crate::extended::characters::test_globals();
        struct Unwind;
        impl Memory for Unwind {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("synthetic character acquisition unwind");
            }
        }
        let mut world = World::new(8);
        let pointer = World::unit(5).unwrap();
        let mut job = CharactersJob::new_part(0);
        acquire(&world, &mut job, true, true);
        world.patch(
            pointer + cunit_leader::EXPERIENCE,
            &(-987_i64).to_le_bytes(),
        );
        for _ in 0..2 {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    acquire(&Unwind, &mut job, false, false)
                }))
                .is_err()
            );
        }
        let recovered = acquire(&world, &mut job, false, false);
        assert_eq!(xp(&recovered, 2_000_005), -987);
    }

    #[test]
    fn daily_xp_refresh_revokes_samples_even_when_the_worker_does_not_start() {
        let _globals = crate::extended::characters::test_globals();
        let mut world = World::new(8);
        let pointer = World::unit(5).unwrap();
        let mut job = CharactersJob::new_part(0);
        acquire(&world, &mut job, true, true);
        let mut rejected = Captured::with_capacities(&capacities());
        for (hour, full, value) in [(12, false, -17_i64), (13, true, i64::MIN)] {
            world.patch(pointer + cunit_leader::EXPERIENCE, &value.to_le_bytes());
            rejected.prepare_hour(hour, full);
            let recovered = acquire(&world, &mut job, false, false);
            assert_eq!(xp(&recovered, 2_000_005), value);
        }
    }

    #[test]
    fn puts_out_of_key_order_are_found_and_settle_to_the_last_value_put() {
        let _globals = crate::extended::characters::test_globals();
        let mut store = Store::default();
        store.begin((1, 2), true, false);
        for (key, value) in [
            ((30, 1, 1), 3),
            ((10, 1, 1), 1),
            ((20, 1, 1), 2),
            ((10, 1, 1), 11),
        ] {
            store.put(key, value);
        }
        assert_eq!(store.get((10, 1, 1)), Some(11));
        assert_eq!(store.get((20, 1, 1)), Some(2));
        assert_eq!(store.get((40, 1, 1)), None);
        store.finish(true);
        let held: Vec<_> = store.entries.iter().map(|entry| (entry.0, entry.1)).collect();
        assert_eq!(held, [((10, 1, 1), 11), ((20, 1, 1), 2), ((30, 1, 1), 3)]);
        store.begin((1, 2), false, false);
        assert_eq!(store.get((30, 1, 1)), Some(3));
        store.put((5, 1, 1), 5);
        store.touch(5, 1);
        store.finish(true);
        assert_eq!(store.entries.len(), 2, "untouched held entries leave");
    }

    #[test]
    fn daily_xp_removal_and_failed_capture_forget_old_occurrences() {
        let _globals = crate::extended::characters::test_globals();
        let mut store = Store::default();
        let key = (100, 200, 300);
        store.begin((1, 2), true, true);
        store.put(key, i64::MIN);
        store.finish(true);
        store.begin((1, 2), false, false);
        assert_eq!(store.get(key), Some(i64::MIN));
        store.finish(true);
        store.begin((1, 2), false, false);
        store.finish(true);
        store.begin((1, 2), false, false);
        assert_eq!(store.get(key), None);
        store.put(key, i64::MAX);
        store.finish(false);
        store.begin((1, 2), false, false);
        assert_eq!(store.get(key), None);
        store.put(key, -5);
        store.finish(true);
        let old_generation = store.generation;
        store.begin((1, 2), false, true);
        assert_ne!(store.generation, old_generation);
        assert_eq!(store.get(key), None, "daily refresh is fresh");
        store.put(key, 17);
        store.finish(true);
        store.forget();
        assert_eq!(store.generation, 0);
        store.begin((1, 2), false, false);
        assert_eq!(
            store.get(key),
            None,
            "an unwound loan leaves no valid sample"
        );
    }
}
