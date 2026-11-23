use collector_states::acquisition_gather::{Report as GatherReport, Request as GatherRequest};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use engine_native_intelligence::country_memory::Cache;

pub(crate) struct Request {
    countries: engine_allocation::SharedBox<Cache>,
    rows: Vec<ccountry::Row>,
    links: GatherRequest<Vec<ccountry::LinkRow>>,
    naming: Option<Naming>,
}
struct Naming {
    appearance: collector_country_counters::country_appearance::Captured,
    hour: i32,
    store: Option<Box<collector_country_counters::country_appearance::AppearanceStore>>,
}

#[derive(Clone, Copy)]
pub(crate) struct Report {
    pub countries: Outcome,
    pub links: GatherReport,
    pub(crate) countries_ticks: i64,
    pub(crate) links_ticks: i64,
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
        countries: &engine_allocation::SharedBox<Cache>,
        base: usize,
        game_state: usize,
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
    ) -> Self {
        Self {
            countries: engine_allocation::SharedBox::clone(countries),
            rows: std::mem::take(rows),
            links: GatherRequest::take(base, game_state, links),
            naming: None,
        }
    }

    pub(crate) fn restore(self, rows: &mut Vec<ccountry::Row>, links: &mut Vec<ccountry::LinkRow>) {
        assert!(
            self.naming.is_none(),
            "country naming owners require matching restore"
        );
        *rows = self.rows;
        self.links.restore(links);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_with_naming(
        countries: &engine_allocation::SharedBox<Cache>,
        base: usize,
        game_state: usize,
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
        appearance: &mut collector_country_counters::country_appearance::Captured,
        hour: i32,
        store: Option<Box<collector_country_counters::country_appearance::AppearanceStore>>,
    ) -> Self {
        let mut request = Self::take(countries, base, game_state, rows, links);
        request.naming = Some(Naming {
            appearance: std::mem::replace(
                appearance,
                collector_country_counters::country_appearance::Captured::vacant(),
            ),
            hour,
            store,
        });
        request
    }
    pub(crate) fn has_naming(&self) -> bool {
        self.naming.is_some()
    }
    pub(crate) fn fail_naming(&mut self, reason: &'static str) -> Report {
        if let Some(naming) = &mut self.naming {
            naming.appearance.fail_preacquired(
                self.links.base,
                self.links.game_state,
                naming.hour,
                reason,
            );
            if let Some(store) = naming.store.as_deref_mut() {
                store.forget();
            }
        }
        Report {
            countries: Outcome::Unavailable { reason },
            links: GatherReport {
                result: Outcome::Unavailable { reason },
            },
            countries_ticks: 0,
            links_ticks: 0,
        }
    }
    pub(crate) fn restore_with_naming(
        mut self,
        rows: &mut Vec<ccountry::Row>,
        links: &mut Vec<ccountry::LinkRow>,
        appearance: &mut collector_country_counters::country_appearance::Captured,
        failure: Option<&'static str>,
    ) -> Option<Box<collector_country_counters::country_appearance::AppearanceStore>> {
        if let Some(reason) = failure {
            self.fail_naming(reason);
        }
        let mut store = None;
        if let Some(naming) = self.naming.take() {
            *appearance = naming.appearance;
            store = naming.store;
        }
        self.restore(rows, links);
        store
    }

    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        let _countries_reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::Countries);
        let cached = self.countries.view(memory);
        let base = self.links.base;
        let game_state = self.links.game_state;
        let begin = collector_primitives::clock::qpc();
        if let Some(naming) = &mut self.naming {
            naming.appearance.raw.begin(naming.hour);
            if let engine_native_intelligence::country_memory::SeedStatus::Unavailable { reason } =
                cached.seed_status()
            {
                naming.appearance.raw.link_controls(Err(reason));
                naming.appearance.raw.table(
                    engine_native_memory::raw_country_observations::Source::Countries,
                    Outcome::Unavailable { reason },
                );
            }
        }
        let countries = match &mut self.naming {
            Some(naming) => ccountry::walk_observed(
                &cached,
                base,
                game_state,
                &mut self.rows,
                &mut naming.appearance.raw,
            ),
            None => ccountry::walk(&cached, base, game_state, &mut self.rows),
        };
        let countries_ticks = collector_primitives::clock::qpc()
            .saturating_sub(begin)
            .max(0);
        drop(_countries_reads);
        let _links_reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::CountryLinks);
        let begin = collector_primitives::clock::qpc();
        let links = self.links.run(|rows| match &mut self.naming {
            Some(naming) => ccountry::walk_links_observed(
                &cached,
                base,
                game_state,
                rows,
                &mut naming.appearance.raw,
            ),
            None => ccountry::walk_links(&cached, base, game_state, rows),
        });
        let links_ticks = collector_primitives::clock::qpc()
            .saturating_sub(begin)
            .max(0);
        drop(_links_reads);
        if let Some(naming) = &mut self.naming {
            naming.appearance.raw.table(
                engine_native_memory::raw_country_observations::Source::Countries,
                countries,
            );
            let carry = naming
                .store
                .as_deref_mut()
                .map(collector_country_counters::country_appearance::Carry::observed);
            naming
                .appearance
                .capture_preacquired(&cached, base, game_state, naming.hour, carry);
            naming.appearance.raw.finish_native();
        }
        Report {
            countries,
            links,
            countries_ticks,
            links_ticks,
        }
    }
}

pub(crate) struct TemplatesRequest {
    pub(crate) workers: crate::acquisition_jobs::variant_workers::Dispatch,
    countries: engine_allocation::SharedBox<Cache>,
    templates: collector_division_templates::extended::division_templates::Captured,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TemplatesReport;

impl TemplatesRequest {
    pub(crate) fn take(
        countries: &engine_allocation::SharedBox<Cache>,
        base: usize,
        game_state: usize,
        templates: &mut collector_division_templates::extended::division_templates::Captured,
        hour: i32,
        force_full: bool,
    ) -> Self {
        Self {
            workers: Default::default(),
            countries: engine_allocation::SharedBox::clone(countries),
            templates: std::mem::replace(
                templates,
                collector_division_templates::extended::division_templates::Captured::vacant(),
            ),
            base,
            game_state,
            hour,
            force_full,
        }
    }

    pub(crate) fn run(&mut self, memory: &impl Memory) -> TemplatesReport {
        let cached = self.countries.view(memory);
        let (workers, base) = (&self.workers, self.base);
        let mut walk = |parts| workers.template_walk(parts, memory, base);
        // SAFETY: Admitted template worker records template sizing demand for the active session.
        unsafe {
            self.templates.capture_preacquired(
                &cached,
                self.base,
                self.game_state,
                self.hour,
                self.force_full,
                Some(
                    collector_division_templates::extended::division_templates::TemplateWorkers {
                        lanes: workers.lanes(),
                        walk: &mut walk,
                    },
                ),
            );
        }
        TemplatesReport
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> TemplatesReport {
        self.templates.fail_preacquired(
            self.base,
            self.game_state,
            self.hour,
            self.force_full,
            reason,
        );
        TemplatesReport
    }

    pub(crate) fn restore(
        mut self,
        templates: &mut collector_division_templates::extended::division_templates::Captured,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure {
            self.fail(reason);
        }
        *templates = self.templates;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::cconvoys;
    use engine_native::cgame_state;
    use engine_native_memory::{memory::SliceMemory, msvc_string};
    use engine_native_politics::cpolitical_status;
    use std::cell::RefCell;

    const STATE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const COUNTRY_TABLE: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const TAGS: usize = 0x2200;
    const STATUS: usize = 0x5000;
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
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + engine_native_parts::cpolitical_status_layout::COUNTRY,
            &(COUNTRY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + engine_native_parts::cpolitical_status_layout::POLITICAL_POWER,
            &(-4_294_967_301_i64).to_le_bytes(),
        );
        bytes
    }

    fn snapshot(bytes: &[u8], cells: usize) -> engine_allocation::SharedBox<Cache> {
        let mut cache =
            engine_allocation::shared(Cache::try_with_capacity(cells).unwrap()).unwrap();
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
        use collector_memory::read_accounting::{self, Capture};
        use engine_native_memory::memory::{CountedMemory, ReadCounts};
        use replay_data::rows::TableId;

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
                let mut storage =
                    engine_allocation::shared(Cache::try_with_capacity(cells).unwrap()).unwrap();
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
                let mut original_storage =
                    engine_allocation::shared(Cache::try_with_capacity(cells).unwrap()).unwrap();
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
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe {
                collector_workers::acquisition::Pool::start(
                    workers,
                    1,
                    move |request: &mut Request| {
                        request.run(&SliceMemory {
                            base: 0,
                            bytes: &bytes,
                        })
                    },
                )
            }
            .unwrap();
            let mut rows = vec![ccountry::Row::default(); 2];
            let mut links = vec![ccountry::LinkRow::default(); 8];
            let pointers = (rows.as_ptr(), links.as_ptr());
            let mut pending = Some(Request::take(&snapshot, 0, STATE, &mut rows, &mut links));
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert_eq!(report.countries, Outcome::Rows { len: 1 });
            assert_eq!(report.links.result, Outcome::Rows { len: 3 });
            completed.request.restore(&mut rows, &mut links);
            assert_eq!((rows.as_ptr(), links.as_ptr()), pointers);
            assert_eq!(engine_allocation::SharedBox::strong_count(&snapshot), 1);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }

    #[test]
    fn named_country_walk_reads_no_retired_convoy_operands() {
        use collector_country_counters::country_appearance;
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let memory = Traced::new(&bytes);
        let mut appearance = country_appearance::Captured::with_capacities(
            &country_appearance::Capacities::with_rows(2),
        );
        let mut rows = vec![ccountry::Row::default(); 2];
        let mut links = vec![ccountry::LinkRow::default(); 8];
        let mut expected = vec![ccountry::Row::default(); 2];
        let expected_outcome = ccountry::walk(&snapshot.view(&memory), 0, STATE, &mut expected);
        memory.calls.borrow_mut().clear();
        let mut request = Request::take_with_naming(
            &snapshot,
            0,
            STATE,
            &mut rows,
            &mut links,
            &mut appearance,
            7,
            None,
        );
        let report = request.run(&memory);
        request.restore_with_naming(&mut rows, &mut links, &mut appearance, None);
        assert_eq!(report.countries, expected_outcome);
        assert_eq!(rows, expected);
        let convoys = COUNTRY + ccountry::CONVOY_STATE;
        for header in [cconvoys::EQUIPMENT_POOL, cconvoys::RESERVATIONS] {
            assert!(
                !memory
                    .calls
                    .borrow()
                    .iter()
                    .any(|&(at, len)| at < convoys + header + 16 && convoys + header < at + len),
                "convoy header {header:#x} read"
            );
        }
    }

    #[test]
    fn the_template_job_preacquires_or_refuses_the_hours_templates() {
        use collector_division_templates::extended::division_templates;
        let bytes = fixture();
        let snapshot = snapshot(&bytes, 256);
        let memory = Traced::new(&bytes);
        let mut templates = division_templates::Captured::vacant();
        let mut request = TemplatesRequest::take(&snapshot, 0, STATE, &mut templates, 7, false);
        request.run(&memory);
        request.restore(&mut templates, None);
        let reads = memory.calls.borrow().len();
        templates.take_preacquired(0, STATE, 7, false);
        assert_eq!(memory.calls.borrow().len(), reads, "taking reads nothing");
        assert_ne!(
            templates.outcomes()[1],
            Outcome::Unavailable {
                reason: "template preacquisition not supplied"
            }
        );

        let reason = "acquisition worker stopped before completion";
        let request = TemplatesRequest::take(&snapshot, 0, STATE, &mut templates, 8, false);
        request.restore(&mut templates, Some(reason));
        assert_eq!(
            templates.take_preacquired(0, STATE, 8, false),
            [0; division_templates::N]
        );
        assert_eq!(templates.outcomes()[1], Outcome::Unavailable { reason });
    }
}
