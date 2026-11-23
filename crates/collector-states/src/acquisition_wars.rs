use collector_primitives::clock::qpc;

use crate::acquisition_gather;
use collector_observers::war_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native::{Memory, ccountry, country_memory::Cache, cwar_relation, walk::Outcome};

pub struct Request {
    pub countries: engine_allocation::SharedBox<Cache>,
    pub table: acquisition_gather::Request<Vec<cwar_relation::Row>>,
    pub snapshot: cwar_relation::OwnedSnapshotScratch,
    pub snapshot_sampled: bool,
    pub store: Option<Box<WarStore>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    token: (u64, u64),
    scope: (usize, usize),
}

#[derive(Default)]
pub struct WarStore {
    rows: Vec<cwar_relation::Row>,
    wars: Vec<usize>,
    countries: Vec<(usize, u32, u32)>,
    key: Option<Key>,
    census: Vec<(usize, u32, u32)>,
    walked: Vec<usize>,
}

impl WarStore {
    pub fn forget(&mut self) {
        self.key = None;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<cwar_relation::Row>()
            + (self.wars.capacity() + self.walked.capacity()) * size_of::<usize>()
            + (self.countries.capacity() + self.census.capacity()) * size_of::<(usize, u32, u32)>()
    }

    fn take_census(&mut self, countries: &impl Memory, base: usize, game_state: usize) -> bool {
        self.census.clear();
        let Ok(reader) = ccountry::Reader::new(countries, base, game_state) else {
            return false;
        };
        if allocation::advisory(|| allocation::reserve(&mut self.census, reader.count() as usize))
            .is_err()
        {
            return false;
        }
        for slot in 1..reader.count() {
            match reader.at(slot) {
                Ok(Some(country)) => {
                    self.census
                        .push((country.pointer, country.canonical_slot, country.id));
                }
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        true
    }

    fn carry(&self, memory: &impl Memory, output: &mut Vec<cwar_relation::Row>) -> bool {
        output.clear();
        if allocation::advisory(|| allocation::reserve(output, self.rows.len())).is_err() {
            return false;
        }
        output.extend_from_slice(&self.rows);
        cwar_relation::read_casualties(memory, &self.wars, output)
    }

    fn hold(&mut self, rows: &[cwar_relation::Row], key: Key) {
        self.key = None;
        self.rows.clear();
        if allocation::advisory(|| allocation::reserve(&mut self.rows, rows.len())).is_err() {
            return;
        }
        self.rows.extend_from_slice(rows);
        std::mem::swap(&mut self.wars, &mut self.walked);
        std::mem::swap(&mut self.countries, &mut self.census);
        self.key = Some(key);
    }

    #[allow(clippy::too_many_arguments)]
    fn capture(
        &mut self,
        memory: &impl Memory,
        countries: &impl Memory,
        base: usize,
        game_state: usize,
        rows: &mut Vec<cwar_relation::Row>,
        snapshot: &mut cwar_relation::OwnedSnapshotScratch,
        changes: &dyn Changes,
    ) -> (Outcome, Counts) {
        let key = changes.quiet().map(|token| Key {
            token,
            scope: (base, game_state),
        });
        let census = self.take_census(countries, base, game_state);
        let matched = key.is_some() && self.key == key && census && self.census == self.countries;
        if matched && self.carry(memory, rows) {
            let len = rows.len();
            let counts = Counts {
                quiet: true,
                ..Counts::default()
            };
            return (Outcome::Rows { len }, counts);
        }
        let outcome = cwar_relation::walk_owned(
            memory,
            countries,
            base,
            game_state,
            rows,
            snapshot,
            Some(&mut self.walked),
        );
        let counts = Counts {
            read: true,
            ..Counts::default()
        };
        match (key, outcome) {
            (Some(key), Outcome::Rows { len })
                if census && self.walked.len() == len && changes.quiet() == Some(key.token) =>
            {
                self.hold(&rows[..len], key);
            }
            _ => self.key = None,
        }
        (outcome, counts)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Report {
    pub table: acquisition_gather::Report,
    pub table_ticks: i64,
}
impl Report {
    pub fn failed(reason: &'static str) -> Self {
        Self {
            table: acquisition_gather::Report {
                result: Outcome::Unavailable { reason },
            },
            table_ticks: 0,
        }
    }
}
impl Request {
    pub fn run(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory)
    }

    fn run_with(&mut self, memory: &impl Memory) -> Report {
        let _reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::WarRelations);
        let Self {
            countries,
            table,
            snapshot,
            snapshot_sampled,
            store,
        } = self;
        let cached = countries.view(memory);
        let (base, game_state) = (table.base, table.game_state);
        let begin = qpc();
        *snapshot_sampled = true;
        let table = table.run(|rows| match store.as_deref_mut() {
            Some(store) => {
                let (outcome, counts) = store.capture(
                    memory,
                    &cached,
                    base,
                    game_state,
                    rows,
                    snapshot,
                    &collector_observers::war_changes::Observer,
                );
                collector_observers::war_changes::record(&counts);
                outcome
            }
            None => {
                cwar_relation::walk_owned(memory, &cached, base, game_state, rows, snapshot, None)
            }
        });
        match table.result {
            Outcome::Rows { len } => collector_observers::event_feed_observer::note_wars(len),
            Outcome::Truncated { len, unread_rows } => {
                collector_observers::event_feed_observer::note_wars(len + unread_rows as usize)
            }
            Outcome::Unavailable { .. } => {}
        }
        Report {
            table,
            table_ticks: qpc() - begin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wars_test_kit as kit;
    use collector_workers::acquisition::Pool;
    use std::cell::RefCell;

    fn request(bytes: &[u8], capacity: usize) -> Request {
        let memory = kit::memory(bytes);
        let mut countries = Cache::try_with_capacity(1024).unwrap();
        assert_eq!(
            countries
                .prepare(&memory, kit::BASE, kit::GAME_STATE)
                .seed_status(),
            engine_native::country_memory::SeedStatus::Ready
        );
        let mut snapshot = cwar_relation::OwnedSnapshotScratch::try_with_capacity(5).unwrap();
        cwar_relation::walk_owned(
            &memory,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            &mut vec![cwar_relation::Row::default(); 4],
            &mut snapshot,
            None,
        );
        snapshot.grow(5).unwrap();
        Request {
            countries: engine_allocation::shared(countries).unwrap(),
            table: acquisition_gather::Request {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                payload: vec![
                    cwar_relation::Row {
                        cancel: 0xa5,
                        ..Default::default()
                    };
                    capacity
                ],
            },
            snapshot,
            snapshot_sampled: false,
            store: None,
        }
    }

    #[test]
    fn single_owned_wars_preserve_primary_rows_from_every_starting_capacity() {
        let bytes = kit::fixture();
        for capacity in [0, 1, 2, 4] {
            let memory = kit::memory(&bytes);
            let mut request = request(&bytes, capacity);
            let mut expected = request.table.payload.clone();
            let baseline = cwar_relation::walk(
                &request.countries.view(&memory),
                kit::BASE,
                kit::GAME_STATE,
                &mut expected,
            );
            let snapshot_capacity = request.snapshot.capacity();
            let report = request.run(&memory);
            assert_eq!(report.table.result, baseline);
            assert_eq!(request.table.payload, expected);
            assert_eq!(request.snapshot.capacity(), snapshot_capacity);
            assert!(request.snapshot_sampled);
            if capacity >= 2 {
                assert_eq!(report.table.result, Outcome::Rows { len: 2 });
            }
        }
    }

    #[test]
    fn growing_owned_wars_fill_the_first_empty_row_buffer() {
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let mut request = request(&bytes, 0);
        let expected = kit::direct(&memory, 2);
        let report = request.run(&memory);
        assert_eq!(report.table.result, Outcome::Rows { len: 2 });
        assert_eq!(report.table.result, expected.owned_outcome);
        assert_eq!(&request.table.payload[..2], &expected.owned);
        assert_eq!(request.table.payload[1].first_country, 3);
        assert_eq!(request.table.payload[1].second_country, 1);
        assert!(request.snapshot_sampled);
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            if self.fault == Some(address) {
                return false;
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }

    #[test]
    fn wars_first_row_fault_is_returned_without_second_observation() {
        let bytes = kit::fixture();
        let memory = Traced {
            bytes: &bytes,
            calls: RefCell::default(),
            fault: Some(kit::WAR1 + cwar_relation::FIRST_CASUALTIES),
        };
        let mut request = request(&bytes, 4);
        let report = request.run(&memory);
        assert!(matches!(report.table.result, Outcome::Unavailable { .. }));
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == kit::WAR1 + cwar_relation::FIRST_CASUALTIES)
                .count(),
            1
        );
    }

    #[test]
    fn wars_payload_and_snapshot_return_through_caller_and_worker_execution() {
        for workers in [0, 2] {
            let bytes = kit::fixture();
            let owned = request(&bytes, 4);
            let pointer = owned.table.payload.as_ptr();
            let capacity = owned.snapshot.capacity();
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe {
                Pool::start(workers, 1, move |request: &mut Request| {
                    request.run(&kit::memory(&bytes))
                })
            }
            .unwrap();
            let mut pending = Some(owned);
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            assert_eq!(
                completion.report.unwrap().table.result,
                Outcome::Rows { len: 2 }
            );
            assert_eq!(completion.request.table.payload.as_ptr(), pointer);
            assert_eq!(completion.request.snapshot.capacity(), capacity);
            assert!(completion.request.snapshot_sampled);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }

    #[derive(Default)]
    struct Domain {
        revision: std::sync::atomic::AtomicU64,
    }
    impl Changes for Domain {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, self.revision.load(std::sync::atomic::Ordering::SeqCst)))
        }
    }
    impl Domain {
        fn write(&self) {
            self.revision
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn capture(
        store: &mut WarStore,
        request: &mut Request,
        memory: &impl Memory,
        domain: &Domain,
    ) -> (Outcome, Counts, Vec<cwar_relation::Row>) {
        let cached = request.countries.view(memory);
        let mut rows = Vec::new();
        let (outcome, counts) = store.capture(
            memory,
            &cached,
            kit::BASE,
            kit::GAME_STATE,
            &mut rows,
            &mut request.snapshot,
            domain,
        );
        (outcome, counts, rows)
    }

    #[test]
    fn a_quiet_hour_reads_only_casualties_and_equals_a_fresh_walk() {
        let mut bytes = kit::fixture();
        let domain = Domain::default();
        let mut request = request(&bytes, 0);
        let mut store = WarStore::default();
        let (outcome, counts, _) = capture(&mut store, &mut request, &kit::memory(&bytes), &domain);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert!(counts.read && !counts.quiet);
        for war in [kit::WAR1, kit::WAR2] {
            let at = war + cwar_relation::SECOND_UNKNOWN_CASUALTIES;
            bytes[at..at + 8].copy_from_slice(&(war as u64).to_le_bytes());
        }
        let memory = Traced {
            bytes: &bytes,
            calls: RefCell::default(),
            fault: None,
        };
        let (outcome, counts, rows) = capture(&mut store, &mut request, &memory, &domain);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert!(counts.quiet && !counts.read);
        let expected = kit::direct(&kit::memory(&bytes), 2);
        assert_eq!(rows, expected.owned);
        assert_eq!(rows[1].second_unknown_casualties, kit::WAR2 as i64);
        let wars = kit::WAR1..kit::RETIRED + 0x1000;
        let reads: Vec<_> = memory
            .calls
            .borrow()
            .iter()
            .copied()
            .filter(|(at, _)| wars.contains(at))
            .collect();
        assert_eq!(
            reads,
            [
                (kit::WAR1 + cwar_relation::FIRST_CASUALTIES, 32),
                (kit::WAR2 + cwar_relation::FIRST_CASUALTIES, 32),
            ]
        );
    }

    #[test]
    fn a_writer_or_a_failed_read_walks() {
        let mut bytes = kit::fixture();
        let domain = Domain::default();
        let mut request = request(&bytes, 0);
        let mut store = WarStore::default();
        let mut hour = |bytes: &[u8]| {
            let (outcome, counts, rows) =
                capture(&mut store, &mut request, &kit::memory(bytes), &domain);
            (outcome, counts.quiet, rows)
        };
        assert!(!hour(&bytes).1);
        assert!(hour(&bytes).1);
        domain.write();
        bytes[kit::WAR2 + engine_native::crelation::RETIRED] = 1;
        let (outcome, quiet, rows) = hour(&bytes);
        assert!(!quiet);
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].first_country, 1);
        assert!(hour(&bytes).1);
        let memory = Traced {
            bytes: &bytes,
            calls: RefCell::default(),
            fault: Some(kit::WAR1 + cwar_relation::FIRST_CASUALTIES),
        };
        let (outcome, counts, _) = capture(&mut store, &mut request, &memory, &domain);
        assert!(counts.read && !counts.quiet);
        assert!(matches!(outcome, Outcome::Unavailable { .. }));
        let (_, counts, _) = capture(&mut store, &mut request, &kit::memory(&bytes), &domain);
        assert!(counts.read);
    }
}
