//! Countries and CountryLinks acquired within the callback's shared country lease.
//! Countries keeps its single walk; only CountryLinks has a comparison window.
//! The callback restores all three row buffers at their original collection point
//! and retains the final shared authentication check before publication.

use crate::acquisition_gather::{Buffers, Report as GatherReport, Request as GatherRequest};
use engine::{Memory, ccountry, country_memory::Cache, walk::Outcome};
use std::sync::Arc;

pub(crate) struct Request {
    countries: Arc<Cache>,
    rows: Vec<ccountry::Row>,
    links: GatherRequest<Vec<ccountry::LinkRow>>,
}

#[derive(Clone, Copy)]
pub(crate) struct Report {
    pub countries: Outcome,
    pub links: GatherReport,
    pub countries_ticks: i64,
    pub links_ticks: i64,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            countries: Outcome::Unavailable {
                reason: "countries acquisition did not complete",
            },
            links: GatherReport::default(),
            countries_ticks: 0,
            links_ticks: 0,
        }
    }
}

impl Request {
    pub(crate) fn take(
        countries: &Arc<Cache>,
        base: usize,
        game_state: usize,
        needed: bool,
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
        links_second: &mut Vec<ccountry::LinkRow>,
    ) -> Self {
        Self {
            countries: Arc::clone(countries),
            rows: std::mem::take(rows),
            links: GatherRequest {
                base,
                game_state,
                needed,
                buffers: Buffers::take(links, links_second),
            },
        }
    }

    pub(crate) fn restore(
        self,
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
        links_second: &mut Vec<ccountry::LinkRow>,
    ) {
        *rows = self.rows;
        self.links.buffers.restore(links, links_second);
    }

    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let cached = self.countries.view(memory);
        let base = self.links.base;
        let game_state = self.links.game_state;
        let begin = crate::sampling::qpc();
        let countries = ccountry::walk(&cached, base, game_state, &mut self.rows);
        let countries_ticks = crate::sampling::qpc().saturating_sub(begin).max(0);
        let begin = crate::sampling::qpc();
        let links = self.links.buffers.rows(self.links.needed, |rows, _| {
            ccountry::walk_links(&cached, base, game_state, rows)
        });
        let links_ticks = crate::sampling::qpc().saturating_sub(begin).max(0);
        Report {
            countries,
            links,
            countries_ticks,
            links_ticks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acquisition,
        sampling::{full_match, mismatches},
    };
    use engine::{cconvoys, cgame_state, cpolitical_status, memory::SliceMemory, msvc_string};
    use std::cell::{Cell, RefCell};

    const STATE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const COUNTRY_TABLE: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const TAGS: usize = 0x2200;
    const STATUS: usize = 0x5000;
    // The country census authenticates links 1, 2 and 7. Link 3 remains a fresh
    // native read on each links walk, including after the first walk fails.
    const FRESH_LINK: usize = LINKS + 3 * 4;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address..address + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x10000];
        for (offset, pointer, count) in [
            (cgame_state::COUNTRIES, COUNTRY_TABLE, 3_u32),
            (cgame_state::COUNTRY_LINKS, LINKS, 8),
            (cgame_state::COUNTRY_TAGS, TAGS, 8),
        ] {
            put(&mut bytes, STATE + offset, &(pointer as u64).to_le_bytes());
            put(&mut bytes, STATE + offset + 8, &count.to_le_bytes());
            put(&mut bytes, STATE + offset + 12, &count.to_le_bytes());
        }
        for slot in [1, 2] {
            put(
                &mut bytes,
                COUNTRY_TABLE + slot * 8,
                &(COUNTRY as u64).to_le_bytes(),
            );
        }
        for id in [1, 2, 7] {
            put(&mut bytes, LINKS + id * 4, &1_u32.to_le_bytes());
        }
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &7_u32.to_le_bytes());
        for (address, text) in [
            (TAGS + 7 * msvc_string::STRIDE, b"D01".as_slice()),
            (COUNTRY + ccountry::NAME, b"N\xff\0ative".as_slice()),
        ] {
            put(&mut bytes, address, text);
            put(
                &mut bytes,
                address + msvc_string::LENGTH,
                &(text.len() as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                address + msvc_string::CAPACITY,
                &msvc_string::INLINE_CAPACITY.to_le_bytes(),
            );
        }
        for (offset, value) in [
            (ccountry::COMMAND_POWER, 4_294_967_297_i64),
            (ccountry::STABILITY, 4_294_967_298),
            (ccountry::WAR_SUPPORT, -4_294_967_299),
            (ccountry::CONVOY_STATE + cconvoys::CACHE_RAW, 4_294_967_300),
        ] {
            put(&mut bytes, COUNTRY + offset, &value.to_le_bytes());
        }
        bytes[COUNTRY + ccountry::CONVOY_STATE + cconvoys::CACHE_VALID] = 1;
        put(
            &mut bytes,
            COUNTRY + ccountry::POLITICAL_STATUS,
            &(STATUS as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS,
            &(cpolitical_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + cpolitical_status::COUNTRY,
            &(COUNTRY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + cpolitical_status::POLITICAL_POWER,
            &(-4_294_967_301_i64).to_le_bytes(),
        );
        bytes
    }

    fn snapshot(bytes: &[u8], cells: usize) -> Arc<Cache> {
        let memory = SliceMemory { base: 0, bytes };
        let mut storage = Arc::new(Cache::with_capacity(cells));
        let snapshot = Cache::prepare_shared(&mut storage, &memory, 0, STATE).unwrap();
        if cells != 0 {
            assert_eq!(
                snapshot.view(&memory).seed_status(),
                engine::country_memory::SeedStatus::Ready
            );
        }
        snapshot
    }

    #[derive(Clone)]
    struct Owned {
        rows: Vec<ccountry::Row>,
        links: Vec<ccountry::LinkRow>,
        second: Vec<ccountry::LinkRow>,
    }

    impl Owned {
        fn new(countries: usize, links: usize) -> Self {
            Self {
                rows: vec![
                    ccountry::Row {
                        id: 91,
                        ..Default::default()
                    };
                    countries
                ],
                links: vec![
                    ccountry::LinkRow {
                        id: 92,
                        canonical_slot: 0
                    };
                    links
                ],
                second: vec![
                    ccountry::LinkRow {
                        id: 93,
                        canonical_slot: 0
                    };
                    links
                ],
            }
        }

        fn take(&mut self, snapshot: &Arc<Cache>, needed: bool) -> Request {
            Request::take(
                snapshot,
                0,
                STATE,
                needed,
                &mut self.rows,
                &mut self.links,
                &mut self.second,
            )
        }

        fn restore(&mut self, request: Request) {
            request.restore(&mut self.rows, &mut self.links, &mut self.second);
        }

        fn storage(&self) -> [(usize, usize, usize); 3] {
            [
                (
                    self.rows.as_ptr() as usize,
                    self.rows.len(),
                    self.rows.capacity(),
                ),
                (
                    self.links.as_ptr() as usize,
                    self.links.len(),
                    self.links.capacity(),
                ),
                (
                    self.second.as_ptr() as usize,
                    self.second.len(),
                    self.second.capacity(),
                ),
            ]
        }
    }

    #[derive(Clone, Copy)]
    enum LinkRead {
        Stable,
        Changed,
        FailFirst,
        FailSecond,
        FailBoth,
        PanicSecond,
    }

    struct Observed<'a> {
        source: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
        links: Cell<usize>,
        action: LinkRead,
        fault: Option<usize>,
    }

    impl<'a> Observed<'a> {
        fn new(bytes: &'a [u8], action: LinkRead) -> Self {
            Self {
                source: SliceMemory { base: 0, bytes },
                reads: RefCell::default(),
                links: Cell::new(0),
                action,
                fault: None,
            }
        }

        fn count(&self, address: usize) -> usize {
            self.reads
                .borrow()
                .iter()
                .filter(|(at, _)| *at == address)
                .count()
        }
    }

    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .fault
                .is_some_and(|fault| (address..address + destination.len()).contains(&fault))
            {
                return false;
            }
            if address == FRESH_LINK && destination.len() == 4 {
                let pass = self.links.get() + 1;
                self.links.set(pass);
                match (self.action, pass) {
                    (LinkRead::FailFirst, 1)
                    | (LinkRead::FailSecond, 2)
                    | (LinkRead::FailBoth, _) => {
                        return false;
                    }
                    (LinkRead::Changed, 2) => {
                        destination.copy_from_slice(&1_u32.to_le_bytes());
                        return true;
                    }
                    (LinkRead::PanicSecond, 2) => panic!("synthetic links second-pass failure"),
                    _ => {}
                }
            }
            self.source.copy(address, destination)
        }
    }

    #[test]
    fn single_countries_walk_and_only_open_links_check_share_cached_authentication() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        for needed in [false, true] {
            let memory = Observed::new(&bytes, LinkRead::Stable);
            let mut owned = Owned::new(2, 8);
            let storage = owned.storage();
            let mut request = owned.take(&snapshot, needed);
            assert!(owned.rows.is_empty() && owned.links.is_empty() && owned.second.is_empty());
            let report = request.run(&memory);
            assert_eq!(report.countries, Outcome::Rows { len: 1 });
            assert_eq!(report.links.result, Outcome::Rows { len: 3 });
            assert_eq!(memory.count(COUNTRY + ccountry::COMMAND_POWER), 1);
            assert_eq!(memory.count(COUNTRY), 0);
            assert_eq!(memory.links.get(), 1 + usize::from(needed));
            assert_eq!(
                full_match(
                    report.links.result,
                    report.links.second,
                    report.links.mismatches
                ),
                needed
            );
            assert!(report.countries_ticks >= 0 && report.links_ticks >= 0);
            owned.restore(request);
            assert_eq!(owned.storage(), storage);
            assert_eq!(owned.rows[0].id, 7);
            assert_eq!(owned.rows[0].canonical_slot, 1);
            assert_eq!(owned.rows[0].tag.as_bytes(), b"D01");
            assert_eq!(owned.rows[0].name.as_bytes(), b"N\xff\0ative");
            assert_eq!(owned.rows[0].political_power_raw, Some(-4_294_967_301));
            assert_eq!(owned.rows[0].command_power_raw, 4_294_967_297);
            assert_eq!(owned.rows[0].stability_raw, 4_294_967_298);
            assert_eq!(owned.rows[0].war_support_raw, -4_294_967_299);
            assert_eq!(owned.rows[0].convoys_raw, Some(4_294_967_300));
            assert_eq!(owned.rows[1].id, 91);
            assert_eq!(owned.second[0].id, if needed { 1 } else { 93 });
            assert_eq!(Arc::strong_count(&snapshot), 1);
        }
    }

    #[test]
    fn owned_walks_match_original_outcomes_bytes_and_capacity_with_or_without_cache() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        for cells in [0, 256] {
            let snapshot = snapshot(&bytes, cells);
            let cached = snapshot.view(&memory);
            for (countries_capacity, links_capacity) in [(0, 0), (1, 1), (2, 2), (2, 8)] {
                for needed in [false, true] {
                    let mut owned = Owned::new(countries_capacity, links_capacity);
                    let mut original = owned.clone();
                    let countries = ccountry::walk(&cached, 0, STATE, &mut original.rows);
                    let links = ccountry::walk_links(&cached, 0, STATE, &mut original.links);
                    let second = needed
                        .then(|| ccountry::walk_links(&cached, 0, STATE, &mut original.second));
                    let mut request = owned.take(&snapshot, needed);
                    let report = request.run(&memory);
                    owned.restore(request);
                    assert_eq!(report.countries, countries);
                    assert_eq!(report.links.result, links);
                    assert_eq!(owned.rows, original.rows);
                    assert_eq!(owned.links, original.links);
                    assert_eq!(owned.second, original.second);
                    if let Some(second) = second {
                        let mismatches =
                            mismatches(links, &original.links, second, &original.second);
                        assert_eq!(report.links.second, second);
                        assert_eq!(report.links.mismatches, mismatches);
                        assert_eq!(full_match(links, second, mismatches), links_capacity >= 3);
                    } else {
                        assert_eq!(report.links.mismatches, 0);
                        assert!(!full_match(
                            report.links.result,
                            report.links.second,
                            report.links.mismatches
                        ));
                    }
                }
            }
        }
    }

    #[test]
    fn shared_authentication_changes_and_faults_are_rejected_at_the_outer_barrier() {
        let original = fixture();
        let snapshot = snapshot(&original, 256);
        for unreadable in [false, true] {
            let mut changed = original.clone();
            put(
                &mut changed,
                COUNTRY + ccountry::COMMAND_POWER,
                &(-8_589_934_599_i64).to_le_bytes(),
            );
            if !unreadable {
                put(&mut changed, COUNTRY, &0_u64.to_le_bytes());
            }
            let mut memory = Observed::new(&changed, LinkRead::Stable);
            memory.fault = unreadable.then_some(COUNTRY);
            let mut owned = Owned::new(2, 8);
            let mut request = owned.take(&snapshot, true);
            let report = request.run(&memory);
            assert_eq!(report.countries, Outcome::Rows { len: 1 });
            assert_eq!(memory.count(COUNTRY), 0);
            assert_eq!(memory.count(COUNTRY + ccountry::COMMAND_POWER), 1);
            assert!(full_match(
                report.links.result,
                report.links.second,
                report.links.mismatches
            ));
            let verification = snapshot.view(&memory).verify();
            assert_eq!(
                verification,
                Err(if unreadable {
                    "country authentication became unreadable during table walks"
                } else {
                    "country authentication changed during table walks"
                })
            );
            // The callback gates every country-dependent check on this verdict.
            assert!(
                !(verification.is_ok()
                    && full_match(
                        report.links.result,
                        report.links.second,
                        report.links.mismatches
                    ))
            );
            owned.restore(request);
            assert_eq!(owned.rows[0].command_power_raw, -8_589_934_599);
        }
    }

    #[test]
    fn links_repeat_after_first_failure_without_crediting_faults_or_changes() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let unavailable = Outcome::Unavailable {
            reason: "country link unreadable or outside link table",
        };
        for (action, first, second, differences) in [
            (
                LinkRead::FailFirst,
                unavailable,
                Outcome::Rows { len: 3 },
                1,
            ),
            (
                LinkRead::FailSecond,
                Outcome::Rows { len: 3 },
                unavailable,
                1,
            ),
            (LinkRead::FailBoth, unavailable, unavailable, 0),
            (
                LinkRead::Changed,
                Outcome::Rows { len: 3 },
                Outcome::Rows { len: 4 },
                1,
            ),
        ] {
            let memory = Observed::new(&bytes, action);
            let mut owned = Owned::new(2, 8);
            let storage = owned.storage();
            let mut request = owned.take(&snapshot, true);
            let report = request.run(&memory);
            assert_eq!(report.countries, Outcome::Rows { len: 1 });
            assert_eq!(memory.links.get(), 2);
            assert_eq!(report.links.result, first);
            assert_eq!(report.links.second, second);
            assert_eq!(report.links.mismatches, differences);
            assert!(!full_match(
                report.links.result,
                report.links.second,
                report.links.mismatches
            ));
            owned.restore(request);
            assert_eq!(owned.storage(), storage);
            assert_eq!(owned.links[0].id, 1);
            assert_eq!(owned.second[0].id, 1);
        }
    }

    #[test]
    fn country_scalar_failure_preserves_independent_links_results_and_checks() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let mut memory = Observed::new(&bytes, LinkRead::Stable);
        memory.fault = Some(COUNTRY + ccountry::COMMAND_POWER);
        let mut owned = Owned::new(2, 8);
        let mut request = owned.take(&snapshot, true);
        let report = request.run(&memory);
        assert_eq!(
            report.countries,
            Outcome::Unavailable {
                reason: "country scalar unreadable"
            }
        );
        assert_eq!(memory.count(COUNTRY + ccountry::COMMAND_POWER), 1);
        assert_eq!(memory.links.get(), 2);
        assert!(full_match(
            report.links.result,
            report.links.second,
            report.links.mismatches
        ));
        assert!(snapshot.view(&memory).verify().is_ok());
        owned.restore(request);
        assert_eq!(owned.rows[0].id, 91);
        assert_eq!(owned.links[0].id, 1);
        assert_eq!(owned.second[0].id, 1);
    }

    #[test]
    fn success_returns_all_buffers_and_snapshot_leases_on_caller_and_workers() {
        for workers in [0, 1, 3] {
            for needed in [false, true] {
                let bytes = fixture();
                let snapshot = snapshot(&bytes, 256);
                let pool = acquisition::Pool::start(workers, 1, move |request: &mut Request| {
                    request.run(&SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    })
                })
                .unwrap();
                let mut owned = Owned::new(2, 8);
                let storage = owned.storage();
                let mut pending = Some(owned.take(&snapshot, needed));
                // SAFETY: synthetic source storage and owned buffers outlive collection.
                let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
                assert!(pending.is_none());
                let completion = pool.finish(ticket).unwrap();
                let report = completion.report.unwrap();
                assert_eq!(report.countries, Outcome::Rows { len: 1 });
                assert_eq!(
                    full_match(
                        report.links.result,
                        report.links.second,
                        report.links.mismatches
                    ),
                    needed
                );
                owned.restore(completion.request);
                assert_eq!(owned.storage(), storage);
                assert_eq!(Arc::strong_count(&snapshot), 1);
                assert_eq!(pool.calibration().uncollected, 0);
            }
        }
    }

    #[test]
    fn rejected_request_retains_all_buffers_and_runs_once_inline() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let pool_bytes = bytes.clone();
        let pool = acquisition::Pool::start(0, 1, move |request: &mut Request| {
            request.run(&SliceMemory {
                base: 0,
                bytes: &pool_bytes,
            })
        })
        .unwrap();
        let mut first = Owned::new(2, 8);
        let mut second = Owned::new(2, 8);
        let first_storage = first.storage();
        let second_storage = second.storage();
        // SAFETY: both requests own their buffers and use retained synthetic storage.
        let ticket = unsafe { pool.submit(&mut Some(first.take(&snapshot, true))) }.unwrap();
        let mut retained = Some(second.take(&snapshot, true));
        assert_eq!(
            unsafe { pool.submit(&mut retained) },
            Err(acquisition::SubmitError::Full)
        );
        let mut request = retained.take().unwrap();
        let memory = Observed::new(&bytes, LinkRead::Stable);
        let report = request.run(&memory);
        assert_eq!(report.countries, Outcome::Rows { len: 1 });
        assert_eq!(memory.count(COUNTRY + ccountry::COMMAND_POWER), 1);
        assert_eq!(memory.links.get(), 2);
        second.restore(request);
        first.restore(pool.finish(ticket).unwrap().request);
        assert_eq!(first.storage(), first_storage);
        assert_eq!(second.storage(), second_storage);
        assert_eq!(Arc::strong_count(&snapshot), 1);
        assert_eq!(pool.calibration().accepted, 1);
        assert_eq!(pool.calibration().rejected, 1);
        assert_eq!(pool.calibration().uncollected, 0);
    }

    #[test]
    fn panic_and_cancellation_return_partial_buffers_without_retry_or_credit() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let pool = acquisition::Pool::start(0, 2, move |request: &mut Request| {
            request.run(&Observed::new(&bytes, LinkRead::PanicSecond))
        })
        .unwrap();
        let mut canceled = Owned::new(2, 8);
        let mut failed = Owned::new(2, 8);
        let canceled_storage = canceled.storage();
        let failed_storage = failed.storage();
        // SAFETY: zero-worker pool retains both requests until these collections.
        let waiting = unsafe { pool.submit(&mut Some(canceled.take(&snapshot, true))) }.unwrap();
        let panicking = unsafe { pool.submit(&mut Some(failed.take(&snapshot, true))) }.unwrap();
        let completion = pool.finish(panicking).unwrap();
        let stopped = pool.finish(waiting).unwrap();
        assert!(matches!(
            completion.report,
            Err(acquisition::Failure::Panicked)
        ));
        assert!(matches!(stopped.report, Err(acquisition::Failure::Stopped)));
        for report in [
            completion.report.unwrap_or_default(),
            stopped.report.unwrap_or_default(),
        ] {
            assert!(matches!(report.countries, Outcome::Unavailable { .. }));
            assert!(!full_match(
                report.links.result,
                report.links.second,
                report.links.mismatches
            ));
            assert_eq!((report.countries_ticks, report.links_ticks), (0, 0));
        }
        failed.restore(completion.request);
        canceled.restore(stopped.request);
        assert_eq!(failed.storage(), failed_storage);
        assert_eq!(canceled.storage(), canceled_storage);
        assert_eq!(failed.rows[0].id, 7);
        assert_eq!(failed.links[2].id, 7);
        assert_eq!(failed.second[1].id, 2);
        assert_eq!(failed.second[2].id, 93);
        assert_eq!(canceled.rows[0].id, 91);
        assert_eq!(canceled.links[0].id, 92);
        assert_eq!(canceled.second[0].id, 93);
        assert_eq!(Arc::strong_count(&snapshot), 1);
        let calibration = pool.calibration();
        assert_eq!(calibration.panicked, 1);
        assert_eq!(calibration.stopped, 1);
        assert_eq!(calibration.caller_executed, 1);
        assert_eq!(calibration.uncollected, 0);
    }
}
