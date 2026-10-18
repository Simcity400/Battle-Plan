//! Owned war census within the callback's current country lease.
//!
//! The rows are held between hours in a [`WarStore`] that travels with the
//! job ([`crate::war_changes`]). An hour is quiet when the war domain's
//! token, the scope and the callback's authenticated country census (which
//! costs no read) equal those taken before the latest complete walk: no
//! forwarded writer of the war set, its vectors or its fixed fields ran
//! since, so that walk's rows and war pointers still stand, and the hour
//! publishes them with each war's casualty counters read afresh
//! ([`cwar_relation::read_casualties`]), one guarded copy per batch of wars,
//! without validating any war. The domain's hour of the rotation
//! ([`crate::sampled_verify`], keyed by the table; every quiet hour under
//! `war_verify`) walks instead and compares the walk with what it would
//! carry; a difference poisons the domain. A walk is held only when it
//! completed, recorded every row's war and the token stood still across it;
//! a failed job forgets the store, and a casualty read that fails walks.
//! Allocation is advisory: a store that cannot grow holds nothing.

use crate::{
    acquisition_gather,
    sampled_verify::Check,
    sampling,
    war_changes::{Changes, Counts},
};
use engine_allocation as allocation;
use engine_native::{Memory, ccountry, country_memory::Cache, cwar_relation, walk::Outcome};
use std::sync::Arc;

pub(crate) struct Request {
    pub countries: Arc<Cache>,
    pub table: acquisition_gather::Request<Vec<cwar_relation::Row>>,
    pub snapshot: cwar_relation::OwnedSnapshotScratch,
    pub snapshot_sampled: bool,
    /// Held rows ([`WarStore`]); absent walks every hour and holds nothing.
    pub store: Option<Box<WarStore>>,
}

/// The rotation key of the domain's daily comparison.
const ROTATION_KEY: u64 = replay_data::rows::TableId::WarRelations as u64;

/// What a quiet hour must find unchanged since the held walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    token: (u64, u64),
    scope: (usize, usize),
}

/// The latest complete walk ([`crate::war_changes`]).
#[derive(Default)]
pub(crate) struct WarStore {
    /// Its rows and each row's war, in walk order.
    rows: Vec<cwar_relation::Row>,
    wars: Vec<usize>,
    /// Its countries in census order: pointer, canonical slot and id.
    countries: Vec<(usize, u32, u32)>,
    /// The key taken before it; `None` holds nothing.
    key: Option<Key>,
    /// The hour's census, walked wars and carried rows.
    census: Vec<(usize, u32, u32)>,
    walked: Vec<usize>,
    carried: Vec<cwar_relation::Row>,
}

impl WarStore {
    /// A failed job's store certifies nothing.
    pub(crate) fn forget(&mut self) {
        self.key = None;
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        (self.rows.capacity() + self.carried.capacity()) * size_of::<cwar_relation::Row>()
            + (self.wars.capacity() + self.walked.capacity()) * size_of::<usize>()
            + (self.countries.capacity() + self.census.capacity()) * size_of::<(usize, u32, u32)>()
    }

    /// The hour's countries in census order into `census`; false when the
    /// census does not read or cannot be stored.
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

    /// The held rows with each war's casualties read afresh into `output`;
    /// false when they cannot be stored or a war does not read.
    fn carry(&self, memory: &impl Memory, output: &mut Vec<cwar_relation::Row>) -> bool {
        output.clear();
        if allocation::advisory(|| allocation::reserve(output, self.rows.len())).is_err() {
            return false;
        }
        output.extend_from_slice(&self.rows);
        cwar_relation::read_casualties(memory, &self.wars, output)
    }

    /// Holds `rows`, whose wars the walk left in `walked`, with the hour's
    /// census under `key`.
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

    /// One hour's WarRelations into `rows`: the held rows with fresh
    /// casualties on a quiet hour, a walk otherwise.
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
        check: Check,
    ) -> (Outcome, Counts) {
        let key = changes.quiet().map(|token| Key {
            token,
            scope: (base, game_state),
        });
        let census = self.take_census(countries, base, game_state);
        let matched = key.is_some() && self.key == key && census && self.census == self.countries;
        let compare = matched && check.covers(ROTATION_KEY);
        if matched && !compare && self.carry(memory, rows) {
            let len = rows.len();
            let counts = Counts {
                quiet: true,
                ..Counts::default()
            };
            return (Outcome::Rows { len }, counts);
        }
        let mut carried = std::mem::take(&mut self.carried);
        let compare = compare && self.carry(memory, &mut carried);
        let outcome = cwar_relation::walk_owned_growing(
            memory,
            countries,
            base,
            game_state,
            rows,
            snapshot,
            Some(&mut self.walked),
        );
        let mut counts = Counts {
            read: true,
            compared: compare,
            compared_wars: if compare { carried.len() as u64 } else { 0 },
            ..Counts::default()
        };
        // A quiet hour the walk does not reproduce is a write no hook saw.
        if compare
            && (outcome != (Outcome::Rows { len: carried.len() })
                || rows[..carried.len()] != carried[..]
                || self.walked != self.wars)
        {
            counts.mismatches = 1;
            changes.mismatch();
        }
        self.carried = carried;
        match (key, outcome) {
            (Some(key), Outcome::Rows { len })
                if census
                    && self.walked.len() == len
                    && counts.mismatches == 0
                    && changes.quiet() == Some(key.token) =>
            {
                self.hold(&rows[..len], key);
            }
            _ => self.key = None,
        }
        (outcome, counts)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub table: acquisition_gather::Report,
    pub table_ticks: i64,
}
impl Report {
    pub(crate) fn failed(reason: &'static str) -> Self {
        Self {
            table: acquisition_gather::Report {
                result: Outcome::Unavailable { reason },
            },
            table_ticks: 0,
        }
    }
}
impl Request {
    #[cfg(test)]
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, false)
    }

    pub(crate) fn run_growing(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, true)
    }

    fn run_with(&mut self, memory: &impl Memory, growing: bool) -> Report {
        let _reads = crate::read_accounting::table(replay_data::rows::TableId::WarRelations);
        let Self {
            countries,
            table,
            snapshot,
            snapshot_sampled,
            store,
        } = self;
        let cached = countries.view(memory);
        let (base, game_state) = (table.base, table.game_state);
        let begin = sampling::qpc();
        *snapshot_sampled = true;
        let table = table.run(|rows| match (growing, store.as_deref_mut()) {
            (true, Some(store)) => {
                let (outcome, counts) = store.capture(
                    memory,
                    &cached,
                    base,
                    game_state,
                    rows,
                    snapshot,
                    &crate::war_changes::Observer,
                    crate::sampled_verify::current_burned_in(crate::war_changes::VERIFY),
                );
                crate::war_changes::record(&counts);
                outcome
            }
            (true, None) => cwar_relation::walk_owned_growing(
                memory, &cached, base, game_state, rows, snapshot, None,
            ),
            (false, _) => {
                cwar_relation::walk_owned(memory, &cached, base, game_state, rows, snapshot)
            }
        });
        // One conference can end every live war relation in an hour.
        match table.result {
            Outcome::Rows { len } => crate::event_feed_observer::note_wars(len),
            Outcome::Truncated { len, unread_rows } => {
                crate::event_feed_observer::note_wars(len + unread_rows as usize)
            }
            Outcome::Unavailable { .. } => {}
        }
        Report {
            table,
            table_ticks: sampling::qpc() - begin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{acquisition::Pool, sampling::wars_test_kit as kit};
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
            &mut [cwar_relation::Row::default(); 4],
            &mut snapshot,
        );
        snapshot.grow(5).unwrap();
        Request {
            countries: Arc::new(countries),
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
    fn single_owned_wars_preserve_primary_rows_and_storage_at_every_capacity() {
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
            let pointer = request.table.payload.as_ptr();
            let snapshot_capacity = request.snapshot.capacity();
            let report = request.run(&memory);
            assert_eq!(report.table.result, baseline);
            assert_eq!(request.table.payload, expected);
            assert_eq!(request.table.payload.as_ptr(), pointer);
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
        assert_eq!(
            request(&bytes, 0).run(&memory).table.result,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        let mut request = request(&bytes, 0);
        let expected = kit::direct(&memory, 2);
        let report = request.run_growing(&memory);
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
            let pool = Pool::start(workers, 1, move |request: &mut Request| {
                request.run(&kit::memory(&bytes))
            })
            .unwrap();
            let mut pending = Some(owned);
            // SAFETY: synthetic source and owned outputs survive until collection.
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

    /// The war domain for a store test: a token the test moves and the
    /// mismatches the store reported.
    #[derive(Default)]
    struct Domain {
        revision: std::sync::atomic::AtomicU64,
        mismatches: std::sync::atomic::AtomicU64,
    }
    impl Changes for Domain {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, self.revision.load(std::sync::atomic::Ordering::SeqCst)))
        }
        fn mismatch(&self) {
            self.mismatches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        check: Check,
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
            check,
        );
        (outcome, counts, rows)
    }

    /// A quiet hour publishes the held walk with each war's casualties read
    /// afresh, reading nothing else of any war; the walk's rows are those a
    /// fresh walk of the same memory gives.
    #[test]
    fn a_quiet_hour_reads_only_casualties_and_equals_a_fresh_walk() {
        let mut bytes = kit::fixture();
        let domain = Domain::default();
        let mut request = request(&bytes, 0);
        let mut store = WarStore::default();
        let (outcome, counts, _) = capture(
            &mut store,
            &mut request,
            &kit::memory(&bytes),
            &domain,
            Check::Nothing,
        );
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
        let (outcome, counts, rows) =
            capture(&mut store, &mut request, &memory, &domain, Check::Nothing);
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

    /// A forwarded writer or an unreadable war walks; a complete walk is held
    /// again for the next hour.
    #[test]
    fn a_writer_or_a_failed_read_walks() {
        let mut bytes = kit::fixture();
        let domain = Domain::default();
        let mut request = request(&bytes, 0);
        let mut store = WarStore::default();
        let mut hour = |bytes: &[u8]| {
            let (outcome, counts, rows) = capture(
                &mut store,
                &mut request,
                &kit::memory(bytes),
                &domain,
                Check::Nothing,
            );
            (outcome, counts.quiet, rows)
        };
        assert!(!hour(&bytes).1);
        assert!(hour(&bytes).1);
        // Relation replacement retires WAR2 and unlinks it from its owner.
        domain.write();
        bytes[kit::WAR2 + engine_native::crelation::RETIRED] = 1;
        let (outcome, quiet, rows) = hour(&bytes);
        assert!(!quiet);
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].first_country, 1);
        assert!(hour(&bytes).1);
        // An unreadable war's casualties walk, and the walk refuses it.
        let memory = Traced {
            bytes: &bytes,
            calls: RefCell::default(),
            fault: Some(kit::WAR1 + cwar_relation::FIRST_CASUALTIES),
        };
        let (outcome, counts, _) =
            capture(&mut store, &mut request, &memory, &domain, Check::Nothing);
        assert!(counts.read && !counts.quiet);
        assert!(matches!(outcome, Outcome::Unavailable { .. }));
        assert_eq!(
            domain.mismatches.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        // A failed walk holds nothing.
        let (_, counts, _) = capture(
            &mut store,
            &mut request,
            &kit::memory(&bytes),
            &domain,
            Check::Nothing,
        );
        assert!(counts.read);
    }

    /// The domain's hour of the rotation walks a quiet hour and compares it
    /// with what it would carry: an equal walk is held, a write no hook saw
    /// is a mismatch that publishes the walk and holds nothing.
    #[test]
    fn the_rotation_hour_compares_and_a_missed_write_poisons() {
        let mut bytes = kit::fixture();
        let domain = Domain::default();
        let mut request = request(&bytes, 0);
        let mut store = WarStore::default();
        let hour = (0..24)
            .find(|&hour| Check::Slice { hour }.covers(ROTATION_KEY))
            .unwrap();
        let rotation = Check::Slice { hour };
        let other = Check::Slice { hour: hour + 1 };
        let mut run = |bytes: &[u8], check| {
            capture(
                &mut store,
                &mut request,
                &kit::memory(bytes),
                &domain,
                check,
            )
        };
        run(&bytes, other);
        let (outcome, counts, _) = run(&bytes, rotation);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert!(counts.read && counts.compared && !counts.quiet);
        assert_eq!((counts.compared_wars, counts.mismatches), (2, 0));
        assert!(run(&bytes, other).1.quiet);
        // An unobserved write of a fixed field.
        bytes[kit::WAR2 + engine_native::crelation::CANCEL] = 9;
        let (outcome, counts, rows) = run(&bytes, Check::Every);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(rows[1].cancel, 9);
        assert_eq!(counts.mismatches, 1);
        assert!(!run(&bytes, other).1.quiet);
        assert_eq!(
            domain.mismatches.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }
}
