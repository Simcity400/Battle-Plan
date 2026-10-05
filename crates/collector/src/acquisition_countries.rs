//! Countries and CountryLinks acquired once within the callback's country lease.

use crate::acquisition_gather::{Report as GatherReport, Request as GatherRequest};
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
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
    ) -> Self {
        Self {
            countries: Arc::clone(countries),
            rows: std::mem::take(rows),
            links: GatherRequest::take(base, game_state, links),
        }
    }

    pub(crate) fn restore(self, rows: &mut Vec<ccountry::Row>, links: &mut Vec<ccountry::LinkRow>) {
        *rows = self.rows;
        self.links.restore(links);
    }

    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let _countries_reads = crate::read_accounting::table(replay::rows::TableId::Countries);
        let cached = self.countries.view(memory);
        let base = self.links.base;
        let game_state = self.links.game_state;
        let begin = crate::sampling::qpc();
        let countries = ccountry::walk(&cached, base, game_state, &mut self.rows);
        let countries_ticks = crate::sampling::qpc().saturating_sub(begin).max(0);
        let _links_reads = crate::read_accounting::table(replay::rows::TableId::CountryLinks);
        let begin = crate::sampling::qpc();
        let links = self
            .links
            .run(|rows| ccountry::walk_links(&cached, base, game_state, rows));
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
    use crate::acquisition;
    use engine::{cconvoys, cgame_state, cpolitical_status, memory::SliceMemory, msvc_string};
    use std::cell::RefCell;

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
        let tag = TAGS + 7 * msvc_string::STRIDE;
        put(&mut bytes, tag, b"D01");
        put(&mut bytes, tag + msvc_string::LENGTH, &3_u64.to_le_bytes());
        put(
            &mut bytes,
            tag + msvc_string::CAPACITY,
            &msvc_string::INLINE_CAPACITY.to_le_bytes(),
        );
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
        let mut cache = Arc::new(Cache::try_with_capacity(cells).unwrap());
        Cache::prepare_shared(&mut cache, &SliceMemory { base: 0, bytes }, 0, STATE).unwrap()
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl<'a> Traced<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                bytes,
                calls: RefCell::default(),
                fault: None,
            }
        }
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            if self
                .fault
                .is_some_and(|at| (address..address + output.len()).contains(&at))
            {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    #[test]
    fn accounting_charges_shared_seed_once_and_preserves_single_table_reads() {
        use crate::read_accounting::{self, Capture};
        use engine::memory::{CountedMemory, ReadCounts};
        use replay::rows::TableId;

        let bytes = fixture();
        for cells in [0, 256] {
            for fault in [
                None,
                Some(COUNTRY + ccountry::COMMAND_POWER),
                Some(FRESH_LINK),
            ] {
                let capture = Capture::new();
                let mut traced = Traced::new(&bytes);
                traced.fault = fault;
                let memory = CountedMemory::new(traced);
                let mut storage = Arc::new(Cache::try_with_capacity(cells).unwrap());
                let snapshot = read_accounting::with_table(TableId::Countries, || {
                    Cache::prepare_shared(&mut storage, &memory, 0, STATE).unwrap()
                });
                let seed = read_accounting::snapshot()[TableId::Countries.index()];
                if cells != 0 {
                    assert!(seed.spans > 0 && seed.bytes > 0);
                }
                let mut rows = vec![ccountry::Row::default(); 2];
                let mut links = vec![ccountry::LinkRow::default(); 8];
                let pointers = (rows.as_ptr(), links.as_ptr());
                let mut request = Request::take(&snapshot, 0, STATE, &mut rows, &mut links);
                let report = request.run(&memory);
                request.restore(&mut rows, &mut links);
                let actual = capture.finish();

                let mut original = Traced::new(&bytes);
                original.fault = fault;
                let mut original_storage = Arc::new(Cache::try_with_capacity(cells).unwrap());
                let original_snapshot =
                    Cache::prepare_shared(&mut original_storage, &original, 0, STATE).unwrap();
                let cached = original_snapshot.view(&original);
                let mut expected_rows = vec![ccountry::Row::default(); 2];
                let mut expected_links = vec![ccountry::LinkRow::default(); 8];
                assert_eq!(
                    ccountry::walk(&cached, 0, STATE, &mut expected_rows),
                    report.countries
                );
                assert_eq!(
                    ccountry::walk_links(&cached, 0, STATE, &mut expected_links),
                    report.links.result
                );
                assert_eq!(rows, expected_rows);
                assert_eq!(links, expected_links);
                assert_eq!((rows.as_ptr(), links.as_ptr()), pointers);
                assert_eq!(*memory.0.calls.borrow(), *original.calls.borrow());
                assert!(actual[TableId::Countries.index()].spans >= seed.spans);
                assert!(actual[TableId::CountryLinks.index()].spans > 0);

                let mut total = ReadCounts::ZERO;
                for (index, counts) in actual.iter().enumerate() {
                    total += *counts;
                    if index != TableId::Countries.index() && index != TableId::CountryLinks.index()
                    {
                        assert_eq!(*counts, ReadCounts::ZERO);
                    }
                }
                let trace = memory.0.calls.borrow();
                assert_eq!(
                    total.spans,
                    trace.iter().filter(|(_, len)| *len != 0).count() as u64
                );
                assert_eq!(
                    total.bytes,
                    trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
                );
            }
        }
    }

    #[test]
    fn owned_country_tables_match_single_serial_rows_and_traces_at_every_capacity() {
        let bytes = fixture();
        for cells in [0, 256] {
            let snapshot = snapshot(&bytes, cells);
            for (country_capacity, link_capacity) in [(0, 0), (1, 1), (2, 2), (2, 8)] {
                let expected = Traced::new(&bytes);
                let cached = snapshot.view(&expected);
                let mut country_rows = vec![ccountry::Row::default(); country_capacity];
                let mut link_rows = vec![ccountry::LinkRow::default(); link_capacity];
                let countries = ccountry::walk(&cached, 0, STATE, &mut country_rows);
                let links = ccountry::walk_links(&cached, 0, STATE, &mut link_rows);
                let actual = Traced::new(&bytes);
                let mut rows = vec![ccountry::Row::default(); country_capacity];
                let mut output = vec![ccountry::LinkRow::default(); link_capacity];
                let pointers = (rows.as_ptr(), output.as_ptr());
                let mut request = Request::take(&snapshot, 0, STATE, &mut rows, &mut output);
                let report = request.run(&actual);
                request.restore(&mut rows, &mut output);
                assert_eq!((report.countries, report.links.result), (countries, links));
                assert_eq!((rows.as_ptr(), output.as_ptr()), pointers);
                assert_eq!(rows, country_rows);
                assert_eq!(output, link_rows);
                assert_eq!(*actual.calls.borrow(), *expected.calls.borrow());
                if cells != 0 && country_capacity == 2 && link_capacity == 8 {
                    assert_eq!(report.countries, Outcome::Rows { len: 1 });
                    assert_eq!(report.links.result, Outcome::Rows { len: 3 });
                    assert_eq!(rows[0].id, 7);
                    assert_eq!(rows[0].canonical_slot, 1);
                    assert_eq!(
                        actual
                            .calls
                            .borrow()
                            .iter()
                            .filter(|(at, _)| *at == FRESH_LINK)
                            .count(),
                        1
                    );
                    assert_eq!(
                        actual
                            .calls
                            .borrow()
                            .iter()
                            .filter(|(at, _)| *at == COUNTRY + ccountry::COMMAND_POWER)
                            .count(),
                        1
                    );
                }
            }
        }
    }

    #[test]
    fn failed_country_scalar_keeps_independent_links_and_first_read_trace() {
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let mut actual = Traced::new(&bytes);
        actual.fault = Some(COUNTRY + ccountry::COMMAND_POWER);
        let mut rows = vec![ccountry::Row::default(); 2];
        let mut links = vec![ccountry::LinkRow::default(); 8];
        let mut request = Request::take(&snapshot, 0, STATE, &mut rows, &mut links);
        let report = request.run(&actual);
        request.restore(&mut rows, &mut links);
        assert!(matches!(report.countries, Outcome::Unavailable { .. }));
        assert_eq!(report.links.result, Outcome::Rows { len: 3 });
        assert_eq!(
            actual
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == COUNTRY + ccountry::COMMAND_POWER)
                .count(),
            1
        );
        assert_eq!(
            actual
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == FRESH_LINK)
                .count(),
            1
        );
    }

    #[test]
    fn pool_returns_single_country_payload_and_country_lease() {
        for workers in [0, 1, 3] {
            let bytes = fixture();
            let snapshot = snapshot(&bytes, 256);
            let pool = acquisition::Pool::start(workers, 1, move |request: &mut Request| {
                request.run(&SliceMemory {
                    base: 0,
                    bytes: &bytes,
                })
            })
            .unwrap();
            let mut rows = vec![ccountry::Row::default(); 2];
            let mut links = vec![ccountry::LinkRow::default(); 8];
            let pointers = (rows.as_ptr(), links.as_ptr());
            let mut pending = Some(Request::take(&snapshot, 0, STATE, &mut rows, &mut links));
            // SAFETY: the runner owns the synthetic memory until collection.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert_eq!(report.countries, Outcome::Rows { len: 1 });
            assert_eq!(report.links.result, Outcome::Rows { len: 3 });
            completed.request.restore(&mut rows, &mut links);
            assert_eq!((rows.as_ptr(), links.as_ptr()), pointers);
            assert_eq!(Arc::strong_count(&snapshot), 1);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }
}
