use super::*;

pub(super) struct FixtureRegion {
    bytes: Vec<u8>,
    base_offset: usize,
    len: usize,
}

impl FixtureRegion {
    pub(super) fn with_alignment(len: usize, align: usize) -> FixtureRegion {
        assert!(align.is_power_of_two(), "alignment must be a power of two");
        assert!(len > 0, "a zero-length fixture region cannot be staged");
        let bytes = vec![0u8; len.saturating_add(align)];
        let addr = bytes.as_ptr() as usize;
        let base_offset = addr.wrapping_neg() % align;
        FixtureRegion {
            bytes,
            base_offset,
            len,
        }
    }

    pub(super) fn new(len: usize) -> FixtureRegion {
        FixtureRegion::with_alignment(len, 8)
    }

    pub(super) fn addr(&self) -> usize {
        self.bytes.as_ptr() as usize + self.base_offset
    }

    pub(super) fn at(&self, offset: usize) -> usize {
        assert!(
            offset < self.len,
            "fixture offset {offset:#x} is outside the staged region (len {:#x})",
            self.len
        );
        self.addr() + offset
    }

    fn window(&mut self, offset: usize, len: usize) -> &mut [u8] {
        let end = offset
            .checked_add(len)
            .expect("fixture write does not overflow");
        assert!(
            end <= self.len,
            "fixture write [{offset:#x}, {end:#x}) is outside the staged region (len {:#x})",
            self.len
        );
        let start = self.base_offset + offset;
        &mut self.bytes[start..start + len]
    }

    pub(super) fn write_bytes(&mut self, offset: usize, value: &[u8]) {
        self.window(offset, value.len()).copy_from_slice(value);
    }

    pub(super) fn write_u8(&mut self, offset: usize, value: u8) {
        self.write_bytes(offset, &[value]);
    }

    pub(super) fn write_i32(&mut self, offset: usize, value: i32) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    pub(super) fn write_i64(&mut self, offset: usize, value: i64) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    pub(super) fn write_u32(&mut self, offset: usize, value: u32) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    pub(super) fn write_u64(&mut self, offset: usize, value: u64) {
        self.write_bytes(offset, &value.to_le_bytes());
    }
}

pub(super) struct ModuleImageWindow {
    region: FixtureRegion,
    window_lo: usize,
    module_base: usize,
}

const FIXTURE_PAGE: usize = 0x1000;

impl ModuleImageWindow {
    pub(super) fn spanning(rvas: &[usize], slack: usize) -> ModuleImageWindow {
        let lo = *rvas.iter().min().expect("at least one RVA to stage");
        let hi = rvas
            .iter()
            .max()
            .expect("at least one RVA to stage")
            .checked_add(slack)
            .expect("window end fits");
        let window_lo = lo & !(FIXTURE_PAGE - 1);
        let region = FixtureRegion::with_alignment(hi - window_lo, FIXTURE_PAGE);
        let module_base = region
            .addr()
            .checked_sub(window_lo)
            .expect("the staged block sits above its own window floor");
        ModuleImageWindow {
            region,
            window_lo,
            module_base,
        }
    }

    pub(super) fn module_base(&self) -> usize {
        self.module_base
    }

    fn offset_of(&self, rva: usize) -> usize {
        assert!(
            rva >= self.window_lo,
            "rva {rva:#x} is below the staged window floor {:#x}",
            self.window_lo
        );
        rva - self.window_lo
    }

    pub(super) fn write_u64_at_rva(&mut self, rva: usize, value: u64) {
        let offset = self.offset_of(rva);
        self.region.write_u64(offset, value);
    }

    pub(super) fn write_bytes_at_rva(&mut self, rva: usize, value: &[u8]) {
        let offset = self.offset_of(rva);
        self.region.write_bytes(offset, value);
    }
}

pub(super) struct CollectorStateFixture {
    pub(super) module: ModuleImageWindow,
    pub(super) heap: FixtureRegion,
    date_ptr: u64,
    country_database_ptr: u64,
    country_roster: Option<(&'static str, usize, Vec<CountryObjectRow>)>,
    profile_sentinels_staged: bool,
}

impl CollectorStateFixture {
    pub(super) fn new(rvas: &[usize], slack: usize, heap_len: usize) -> CollectorStateFixture {
        CollectorStateFixture {
            module: ModuleImageWindow::spanning(rvas, slack),
            heap: FixtureRegion::new(heap_len),
            date_ptr: 0,
            country_database_ptr: 0,
            country_roster: None,
            profile_sentinels_staged: false,
        }
    }

    pub(super) fn set_date_ptr_at(&mut self, heap_offset: usize) -> &mut Self {
        self.date_ptr = self.heap.at(heap_offset) as u64;
        self
    }

    pub(super) fn stage_scheduler_clock_at(&mut self, heap_offset: usize) -> &mut Self {
        let game_state = self.heap.at(heap_offset) as u64;
        self.date_ptr = game_state;
        self.module.write_u64_at_rva(
            compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
            game_state,
        );
        self.heap.write_i32(
            heap_offset + GS_CURRENT_DATE_OFFSET,
            fixture_game_hour_raw(),
        );
        self
    }

    pub(super) fn set_verified_country_roster(
        &mut self,
        status: &'static str,
        country_vtable: usize,
        rows: Vec<CountryObjectRow>,
    ) -> &mut Self {
        self.country_roster = Some((status, country_vtable, rows));
        self
    }

    pub(super) fn stage_profile_sentinels(&mut self) -> &mut Self {
        self.module
            .write_bytes_at_rva(C_GAME_STATE_DAILY_UPDATE_OFFSET, &DAILY_UPDATE_SENTINEL);
        self.module
            .write_bytes_at_rva(GET_LONG_STRING_OFFSET, &GET_LONG_STRING_SENTINEL);
        self.profile_sentinels_staged = true;
        self
    }

    pub(super) fn collector_state(&self) -> CollectorState {
        let base = self.module.module_base();
        let country_identity = match &self.country_roster {
            None => CountryIdentityCache::not_attempted("state_fixture"),
            Some((status, country_vtable, rows)) => CountryIdentityCache {
                status,
                scan_ms: 0,
                country_vtable: *country_vtable,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows: rows.clone(),
                rejected_objects: 0,
                duplicate_tags: 0,
                completeness: CountryRosterCompleteness::Complete,
            },
        };
        CollectorState {
            d67_deferred: Mutex::new(D67DeferredRead::default()),
            base,
            daily_addr: base.saturating_add(C_GAME_STATE_DAILY_UPDATE_OFFSET),
            long_string_addr: base.saturating_add(GET_LONG_STRING_OFFSET),
            daily_match: self.profile_sentinels_staged,
            long_string_match: self.profile_sentinels_staged,
            date_ptr: self.date_ptr,
            country_database_ptr: self.country_database_ptr,
            country_identity,
            special_projects: SpecialProjectsCache::not_attempted("state_fixture"),
            weather: CompactWeatherRoot::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FrameRecord {
    pub(super) sequence: u64,
    pub(super) game_hour_raw: i32,
    pub(super) domain_id: u16,
    pub(super) record_type: u16,
    pub(super) record_flags: u32,
    pub(super) payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct DomainPerfView {
    pub(super) elapsed_wall_ms: u64,
    pub(super) read_time_us: u64,
    pub(super) row_count: u32,
    pub(super) changed_rows: u32,
    pub(super) unit_count: u32,
    pub(super) changed_unit_count: u32,
    pub(super) status_flags: u32,
    pub(super) diagnostics: Vec<(String, u64)>,
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

fn le_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

impl DomainPerfView {
    fn parse(payload: &[u8]) -> DomainPerfView {
        let mut diagnostics = Vec::new();
        if payload.len() > 40 && &payload[40..44] == COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
            let unit_count = u16::from_le_bytes([payload[46], payload[47]]) as usize;
            let mut at = 48usize;
            for _ in 0..unit_count {
                let name_len = u16::from_le_bytes([payload[at], payload[at + 1]]) as usize;
                let value = le_u64(payload, at + 4);
                let name =
                    String::from_utf8_lossy(&payload[at + 12..at + 12 + name_len]).to_string();
                diagnostics.push((name, value));
                at += 12 + name_len;
            }
        }
        DomainPerfView {
            elapsed_wall_ms: le_u64(payload, 0),
            read_time_us: le_u64(payload, 8),
            row_count: le_u32(payload, 16),
            changed_rows: le_u32(payload, 20),
            unit_count: le_u32(payload, 24),
            changed_unit_count: le_u32(payload, 28),
            status_flags: le_u32(payload, 32),
            diagnostics,
        }
    }
}

pub(super) struct FrameOutcome {
    pub(super) bytes: Vec<u8>,
    pub(super) records: Vec<FrameRecord>,
    pub(super) next_sequence: u64,
    pub(super) timing: CompactFramePhaseTiming,
    pub(super) trackers: CompactPerfGateTrackers,
}

impl FrameOutcome {
    pub(super) fn record(&self, domain_id: u16, record_type: u16) -> Option<&FrameRecord> {
        self.records
            .iter()
            .find(|record| record.domain_id == domain_id && record.record_type == record_type)
    }

    pub(super) fn domain_data(&self, domain_id: u16) -> Option<&FrameRecord> {
        self.record(domain_id, COMPACT_RECORD_TYPE_DOMAIN_DATA)
    }

    pub(super) fn perf(&self, domain_id: u16) -> Option<DomainPerfView> {
        self.record(domain_id, COMPACT_RECORD_TYPE_DOMAIN_PERF)
            .map(|record| DomainPerfView::parse(&record.payload))
    }

    pub(super) fn assert_diagnostics_agree_with_production(&self, domain_id: u16) {
        let record = self
            .record(domain_id, COMPACT_RECORD_TYPE_DOMAIN_PERF)
            .unwrap_or_else(|| panic!("domain {domain_id} emitted a perf record"));
        let mine = DomainPerfView::parse(&record.payload)
            .diagnostics
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>();
        let production = compact_perf_diagnostic_units(&record.payload)
            .into_iter()
            .map(|unit| format!("{}={}", unit.name, unit.value))
            .collect::<Vec<_>>();
        assert_eq!(
            mine, production,
            "the fixture's PDU decode must agree with the production reader on domain {domain_id}"
        );
    }

    /// The ONE value a domain published under `name` this frame. A name that
    /// occurs twice is a malformed record, and a reader that took the first
    /// would disagree with one that took the last (the audit refuses such a
    /// probe; Codex round 5, finding 2), so it is a failure here, not a pick.
    pub(super) fn diagnostic_unit(&self, domain_id: u16, name: &str) -> Option<u64> {
        let values: Vec<u64> = self
            .records
            .iter()
            .filter(|record| {
                record.domain_id == domain_id
                    && record.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
            })
            .flat_map(|record| {
                DomainPerfView::parse(&record.payload)
                    .diagnostics
                    .into_iter()
                    .filter(|(unit, _)| unit == name)
                    .map(|(_, value)| value)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            values.len() <= 1,
            "domain {domain_id} publishes {name} {} times in one frame: {values:?}",
            values.len()
        );
        values.first().copied()
    }

    pub(super) fn frame_end(&self) -> &FrameRecord {
        self.records
            .last()
            .filter(|record| record.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END)
            .expect("the final record of a frame is its FRAME_END")
    }

    pub(super) fn domains_with_data(&self) -> Vec<u16> {
        let mut ids = self
            .records
            .iter()
            .filter(|record| record.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA)
            .map(|record| record.domain_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub(super) fn lane_span(&self, tag: u16) -> Option<&LaneSpan> {
        self.timing.lane_spans.iter().find(|span| span.tag == tag)
    }
}

pub(super) fn drive_compact_frame(
    state: Option<&CollectorState>,
    control: &CollectorControl,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> FrameOutcome {
    drive_compact_frame_with_deferred_work(state, control, game_hour_raw, elapsed_wall_ms, None)
}

/// [`drive_compact_frame`] with the in-frame publisher's work supplied, so a
/// test can put a publication beside the readers and read what the probe says
/// about it.
pub(super) fn drive_compact_frame_with_deferred_work(
    state: Option<&CollectorState>,
    control: &CollectorControl,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    deferred_work: Option<InFrameDeferredWork<'_>>,
) -> FrameOutcome {
    let mut span_delta_consumer =
        compact_contract::payload_span_delta::PayloadSpanDeltaConsumer::new();
    drive_compact_frame_carrying(
        state,
        control,
        game_hour_raw,
        elapsed_wall_ms,
        CompactPerfGateTrackers::default(),
        &mut span_delta_consumer,
        deferred_work,
    )
}

/// One frame over trackers and a `PSD1` consumer the caller carries, so a
/// test can drive consecutive hours through the same tracker state (the
/// outcome hands the trackers back) and read every delta record against the
/// baseline the producer pinned on an earlier frame.
pub(super) fn drive_compact_frame_carrying(
    state: Option<&CollectorState>,
    control: &CollectorControl,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    mut trackers: CompactPerfGateTrackers,
    span_delta_consumer: &mut compact_contract::payload_span_delta::PayloadSpanDeltaConsumer,
    deferred_work: Option<InFrameDeferredWork<'_>>,
) -> FrameOutcome {
    let _rc_counters = super::super::lock_rc_counters();
    page_cache_new_frame();
    let (bytes, next_sequence, timing) = build_compact_extraction_gate_records(
        state,
        control,
        1,
        elapsed_wall_ms,
        Some(game_hour_raw),
        None,
        None,
        None,
        None,
        None,
        None,
        &mut trackers,
        None,
        deferred_work,
        &mut OutsideBuildSpans::default(),
        &mut FrameBuffers::default(),
    ).expect("test frame build");

    let mut records = Vec::new();
    let mut offset = 0usize;
    // THE FIXTURE IS A STREAM WALKER, so it reconstructs `PSD1` envelopes for
    // itself exactly as the fold, the seal walk, the manifest summary and the
    // completeness audit do. That is not a convenience for the assertions below:
    // it is what makes every d41/d43/d53 test in this crate a live exercise of
    // the round trip, because each one reads a payload that has been through the
    // producer and back.
    while offset < bytes.len() {
        let header = compact_contract::parse_compact_record_header(&bytes[offset..])
            .expect("the frame builder emits well-framed compact records");
        let payload_start = offset + compact_contract::COMPACT_RECORD_HEADER_LEN;
        let payload = bytes[payload_start..payload_start + header.payload_len].to_vec();
        compact_contract::record_semantics::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            &payload,
        )
        .unwrap_or_else(|error| {
            panic!(
                "domain {} record type {} failed the shipped record grammar: {error}",
                header.domain_id, header.record_type
            )
        });
        let reconstructed = compact_contract::payload_span_delta::reconstruct_if_enveloped(
            span_delta_consumer,
            header.record_flags,
            compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                domain_id: header.domain_id,
                source_sequence: header.sequence,
                game_hour_raw: header.game_hour_raw,
            },
            &payload,
        )
        .unwrap_or_else(|error| {
            panic!(
                "domain {} PSD1 record failed to reconstruct: {error}",
                header.domain_id
            )
        });
        let record_flags = if reconstructed.is_some() {
            header.record_flags
                & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
        } else {
            header.record_flags
        };
        records.push(FrameRecord {
            sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            record_flags,
            payload: reconstructed.unwrap_or(payload),
        });
        offset += header.record_len();
    }
    assert_eq!(offset, bytes.len(), "frame bytes end on a record boundary");

    FrameOutcome {
        bytes,
        records,
        next_sequence,
        timing,
        trackers,
    }
}

pub(super) fn scheduler_clock_image() -> ModuleImageWindow {
    ModuleImageWindow::spanning(
        &[compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA],
        0x1000,
    )
}

pub(super) fn bind_scheduler_clock(image: &mut ModuleImageWindow, state: &mut CollectorState) {
    assert!(
        state.date_ptr != 0 && is_heap_pointer_aligned(state.date_ptr as usize, 8),
        "the scheduler clock authenticates date_ptr as a heap pointer; \
         stage the CGameState root before binding the image"
    );
    image.write_u64_at_rva(
        compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        state.date_ptr,
    );
    let base = image.module_base();
    state.base = base;
    state.daily_addr = base.saturating_add(C_GAME_STATE_DAILY_UPDATE_OFFSET);
    state.long_string_addr = base.saturating_add(GET_LONG_STRING_OFFSET);
}

pub(super) fn with_reader_clock_bracket<T>(
    state: &CollectorState, tag: u16, read: impl FnOnce() -> T,
) -> T {
    let mut image = scheduler_clock_image();
    image.write_u64_at_rva(
        compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        state.date_ptr,
    );
    let lanes = FrameLaneTracker::new(
        Instant::now(), Instant::now(), image.module_base(),
        state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
    );
    let result = {
        let _guard = ReaderClockBracketGuard::new(&lanes, tag);
        read()
    };
    let brackets = lanes.clock_brackets.lock().unwrap();
    assert_eq!(brackets.len(), 1);
    let opening = brackets[0].opening.expect("opening scheduler witness");
    let closing = brackets[0].closing.expect("closing scheduler witness");
    assert_eq!(opening.hour, closing.hour);
    assert!(closing.ordinal > opening.ordinal);
    assert!(!brackets[0].worker_panicked);
    result
}

pub(super) fn naval_frame_fixture() -> (Box<AlignedBytes<0x9000>>, CollectorState, ModuleImageWindow)
{
    let mut image = scheduler_clock_image();
    let (mut arena, mut state) = compact_naval_fixture_at_module_base(image.module_base());
    bind_scheduler_clock(&mut image, &mut state);
    let date_offset = (state.date_ptr as usize)
        .checked_sub(arena.addr())
        .expect("the naval date root lives inside its own arena")
        + GS_CURRENT_DATE_OFFSET;
    arena.write_u32(date_offset, fixture_game_hour_raw() as u32);
    (arena, state, image)
}

pub(super) fn clean_status_fixture(heap_len: usize) -> CollectorStateFixture {
    use compact_contract::build_profile as p;

    const COUNTRY_AT: usize = 0x100;
    let mut fixture = CollectorStateFixture::new(
        &[
            C_GAME_STATE_DAILY_UPDATE_OFFSET,
            GET_LONG_STRING_OFFSET,
            p::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        ],
        0x1000,
        heap_len,
    );
    fixture.stage_profile_sentinels();
    let country_ptr = fixture.heap.at(COUNTRY_AT);
    let country_vtable = fixture.module.module_base() + p::DOMAIN57_COUNTRY_VTABLE_RVA;
    fixture.set_verified_country_roster(
        "gamestate_country_table_verified",
        country_vtable,
        vec![test_country_row("POR", 1, country_ptr)],
    );
    fixture
}

pub(super) fn compact_control_scheduling(domains: &[&str]) -> CollectorControl {
    let mut control = raw_test_control();
    control.stream_format = CollectorStreamFormat::Compact;
    control.scheduled_domains = domains.iter().map(|name| (*name).to_string()).collect();
    control
}

pub(super) fn fixture_game_hour_raw() -> i32 {
    compact_contract::calendar::DATE_EPOCH_CONST
        .checked_add(16_959_360i32)
        .expect("fixture hour is representable")
}

#[test]
fn populated_state_makes_the_ships_domain_emit_domain_data_rows_offline() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = compact_control_scheduling(&[RAW_SHIPS_DOMAIN]);
    let outcome = drive_compact_frame(Some(&state), &control, fixture_game_hour_raw(), 7);

    let perf = outcome
        .perf(COMPACT_DOMAIN_SHIPS)
        .expect("d29 emitted its perf record");
    assert_eq!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "the staged ship roots are readable, so d29 must not report SOURCE_UNAVAILABLE; \
         status {:#010x}",
        perf.status_flags
    );
    assert_eq!(perf.row_count, 2, "both staged ships were observed");
    assert_eq!(perf.unit_count, 1, "one staged task force was observed");
    assert_eq!(perf.changed_rows, 2, "both rows are new to a cold tracker");

    let data = outcome
        .domain_data(COMPACT_DOMAIN_SHIPS)
        .expect("d29 emitted a DOMAIN_DATA record");
    let native = with_reader_clock_bracket(&state, COMPACT_DOMAIN_SHIPS, || {
        let roots = discover_compact_naval_oob_roots(&state).expect("staged naval roots");
        sample_compact_ships_from_roots(state.base, &roots, true).expect("ship rows")
    });
    let staged_row = |instance_id: u32, category_enum: u32, sunk_convoys: u32, definition: &str| {
        CompactShipRow {
            instance_id,
            deleted: false,
            task_force_id: Some(617),
            task_force_id_space: Some(OOB_TYPE_FLEET),
            owner_tag_db_id: Some(1),
            definition: Some(definition.to_string()),
            category_enum,
            strength_raw: 10_000_000 + instance_id,
            organisation_raw: 9_000_000 + instance_id,
            experience_raw: 500_000 + instance_id,
            sunk_convoys,
        }
    };
    let staged_rows = vec![
        staged_row(1001, 3, 2, "hull_a"),
        staged_row(1002, 5, 0, "hull_b"),
    ];
    assert_eq!(
        native.rows, staged_rows,
        "the production ship reader must return exactly the staged values"
    );
    let expected = compact_ships_data_payload(
        &staged_rows,
        7,
        perf.status_flags,
        perf.changed_rows,
        perf.row_count,
        native.observed_task_force_rows,
        0,
    );
    assert_eq!(
        data.payload, expected,
        "the frame's d29 payload must be the production encoding of those rows,          with the frame's own hour/wall/status/count scalars"
    );

    assert_eq!(
        outcome.frame_end().record_type,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END
    );
    assert_eq!(outcome.timing.emitted_domains, 1);
    assert_eq!(outcome.domains_with_data(), vec![COMPACT_DOMAIN_SHIPS]);

    assert_eq!(
        outcome
            .records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        (1..=outcome.records.len() as u64).collect::<Vec<_>>()
    );
    assert!(outcome.next_sequence > outcome.records.len() as u64);
    for record in &outcome.records {
        assert_eq!(record.game_hour_raw, fixture_game_hour_raw());
    }
    assert_eq!(
        perf.elapsed_wall_ms, 7,
        "the perf record carries the frame's wall"
    );
    assert_eq!(perf.changed_unit_count, 2);
    assert!(perf.read_time_us <= outcome.timing.frame_time_us);
    let d29_perf_record = outcome
        .record(COMPACT_DOMAIN_SHIPS, COMPACT_RECORD_TYPE_DOMAIN_PERF)
        .expect("d29 perf record");
    assert_ne!(
        d29_perf_record.record_flags
            & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        0,
        "d29 always attaches its PDU1 trailer, so the flag must be set"
    );
    assert_eq!(
        perf.diagnostics,
        vec![
            ("domain29_source_unavailable".to_string(), 0u64),
            ("domain29_invalid_task_force_rows".to_string(), 0u64),
            // Lane SR: the same count under the pre-2026-08-27 rule, in the
            // always-emitted block so a frame where the two AGREE is visible.
            ("domain29_invalid_task_force_rows_old_rule".to_string(), 0u64),
            (
                "naval_oob_roots_discovery_us".to_string(),
                perf_diag_value(&perf, "naval_oob_roots_discovery_us")
            ),
            ("domain29_paid_naval_discovery".to_string(), 1u64),
            // Lane SD2's phase split: the two walls that sum to the discovery,
            // and the walk's population. Always emitted, because a wall is only
            // useful as a per-frame series; the six REFUSAL counters beside them
            // are emitted only when non-zero, and this fixture earns none of
            // them, so none appear.
            (
                "naval_oob_fleet_roots_us".to_string(),
                perf_diag_value(&perf, "naval_oob_fleet_roots_us")
            ),
            (
                "naval_oob_task_force_walk_us".to_string(),
                perf_diag_value(&perf, "naval_oob_task_force_walk_us")
            ),
            (
                "naval_oob_walk_elements".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_elements")
            ),
            (
                "naval_oob_walk_accepted".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_accepted")
            ),
            (
                "naval_oob_walk_engine_total".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_engine_total")
            ),
            (
                "naval_oob_walk_distinct_lifted".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_distinct_lifted")
            ),
        ],
        "the fixture's diagnostic view is the units d29 actually emitted"
    );
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_SHIPS);
}

#[test]
fn a_state_whose_build_profile_is_unverified_makes_d29_typed_source_unavailable() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    let mut fixture =
        CollectorStateFixture::new(&[p::DOMAIN57_CGAMESTATE_SINGLETON_RVA], 0x1000, 0x2000);
    fixture.stage_scheduler_clock_at(0x1000);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_SHIPS_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_SHIPS)
        .expect("d29 emitted its perf record");
    assert_ne!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "unreadable roots must be typed SOURCE_UNAVAILABLE, never silently empty"
    );
    assert_eq!(perf.row_count, 0);
    assert_eq!(
        perf.diagnostics,
        vec![
            ("domain29_source_unavailable".to_string(), 1u64),
            ("domain29_invalid_task_force_rows".to_string(), 0u64),
            // Lane SR: the same count under the pre-2026-08-27 rule, in the
            // always-emitted block so a frame where the two AGREE is visible.
            ("domain29_invalid_task_force_rows_old_rule".to_string(), 0u64),
            (
                "naval_oob_roots_discovery_us".to_string(),
                perf_diag_value(&perf, "naval_oob_roots_discovery_us")
            ),
            ("domain29_paid_naval_discovery".to_string(), 1u64),
            // Lane SD2's phase split: the two walls that sum to the discovery,
            // and the walk's population. Always emitted, because a wall is only
            // useful as a per-frame series; the six REFUSAL counters beside them
            // are emitted only when non-zero, and this fixture earns none of
            // them, so none appear.
            (
                "naval_oob_fleet_roots_us".to_string(),
                perf_diag_value(&perf, "naval_oob_fleet_roots_us")
            ),
            (
                "naval_oob_task_force_walk_us".to_string(),
                perf_diag_value(&perf, "naval_oob_task_force_walk_us")
            ),
            (
                "naval_oob_walk_elements".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_elements")
            ),
            (
                "naval_oob_walk_accepted".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_accepted")
            ),
            (
                "naval_oob_walk_engine_total".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_engine_total")
            ),
            (
                "naval_oob_walk_distinct_lifted".to_string(),
                perf_diag_value(&perf, "naval_oob_walk_distinct_lifted")
            ),
        ]
    );
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_SHIPS);
    assert!(outcome.domain_data(COMPACT_DOMAIN_SHIPS).is_some());
}

#[test]
fn without_a_readable_module_image_every_domain_record_becomes_a_typed_clock_gap() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};

    let (_arena, state) = compact_naval_fixture();
    let outcome = drive_compact_frame(
        Some(&state),
        &compact_control_scheduling(&[RAW_SHIPS_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    assert!(outcome.perf(COMPACT_DOMAIN_SHIPS).is_none());
    assert!(outcome.domain_data(COMPACT_DOMAIN_SHIPS).is_none());
    let gap = outcome
        .record(
            COMPACT_DOMAIN_SHIPS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        )
        .expect("d29's records were replaced by one typed gap");
    let decoded = DomainGapRecordV1::decode(&gap.payload).expect("shipped gap payload");
    assert_eq!(decoded.reason, DomainGapReason::ClockUnreadable);
    assert_eq!(decoded.target_hour_raw, fixture_game_hour_raw());
}

#[test]
fn synthetic_module_image_lets_domain57_read_game_speed_from_base_relative_memory() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    const GAME_STATE_AT: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x2000);
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);

    for staged_speed in 0..=p::DOMAIN57_MAX_GAME_SPEED {
        fixture.heap.write_i32(
            GAME_STATE_AT + p::DOMAIN57_CGAMESTATE_GAME_SPEED_INDEX,
            staged_speed,
        );
        let state = fixture.collector_state();

        let control = compact_control_scheduling(&[RAW_SESSION_MULTIPLAYER_METADATA_LIVE_DOMAIN]);
        let outcome = drive_compact_frame(Some(&state), &control, fixture_game_hour_raw(), 7);

        let data = outcome
            .domain_data(COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE)
            .expect("d57 emitted a DOMAIN_DATA record");
        let frame = outcome
            .trackers
            .d57_terminal_frame
            .as_ref()
            .expect("d57 published its terminal frame");
        assert_eq!(
            data.payload,
            session57::encode_payload(frame),
            "the emitted d57 payload must be the production encoding of the frame it read"
        );

        let speeds = frame
            .event_rows
            .iter()
            .filter(|event| event.kind == session57::SessionEventKind::GameSpeed)
            .map(|event| event.value_i32)
            .collect::<Vec<_>>();
        assert_eq!(
            speeds,
            vec![staged_speed],
            "d57 must carry exactly the game speed staged at \
             base + DOMAIN57_CGAMESTATE_SINGLETON_RVA (+{:#x})",
            p::DOMAIN57_CGAMESTATE_GAME_SPEED_INDEX
        );
    }
}

#[test]
fn an_out_of_range_speed_word_makes_domain57_fail_closed_not_read_zero() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    const GAME_STATE_AT: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x2000);
    fixture.heap.write_i32(
        GAME_STATE_AT + p::DOMAIN57_CGAMESTATE_GAME_SPEED_INDEX,
        p::DOMAIN57_MAX_GAME_SPEED + 1,
    );
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);
    let state = fixture.collector_state();
    let control = compact_control_scheduling(&[RAW_SESSION_MULTIPLAYER_METADATA_LIVE_DOMAIN]);
    let outcome = drive_compact_frame(Some(&state), &control, fixture_game_hour_raw(), 7);

    let frame = outcome
        .trackers
        .d57_terminal_frame
        .as_ref()
        .expect("d57 published its terminal frame");
    assert!(
        !frame
            .event_rows
            .iter()
            .any(|event| event.kind == session57::SessionEventKind::GameSpeed),
        "an out-of-range speed word must yield NO game-speed row, never a coerced 0"
    );
}

#[test]
fn staged_profile_sentinels_satisfy_the_shipped_signature_gate() {
    use compact_contract::build_profile as p;

    let mut fixture = CollectorStateFixture::new(
        &[
            C_GAME_STATE_DAILY_UPDATE_OFFSET,
            GET_LONG_STRING_OFFSET,
            p::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        ],
        0x1000,
        0x2000,
    );

    let unsigned = fixture.collector_state();
    assert!(!sentinel_matches(
        unsigned.daily_addr,
        &DAILY_UPDATE_SENTINEL
    ));
    assert!(!sentinel_matches(
        unsigned.long_string_addr,
        &GET_LONG_STRING_SENTINEL
    ));

    fixture.stage_profile_sentinels();
    let signed = fixture.collector_state();
    assert_eq!(
        signed.daily_addr,
        signed.base + C_GAME_STATE_DAILY_UPDATE_OFFSET,
        "daily_addr is derived from base exactly as CollectorState::resolve derives it"
    );
    assert_eq!(
        signed.long_string_addr,
        signed.base + GET_LONG_STRING_OFFSET
    );
    assert!(
        sentinel_matches(signed.daily_addr, &DAILY_UPDATE_SENTINEL),
        "the staged daily-update signature must satisfy the shipped sentinel read"
    );
    assert!(
        sentinel_matches(signed.long_string_addr, &GET_LONG_STRING_SENTINEL),
        "the staged long-string signature must satisfy the shipped sentinel read"
    );
    assert!(signed.daily_match && signed.long_string_match);
}

#[test]
fn a_staged_date_root_is_read_by_the_frames_own_acceptance_skew_probe() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    const GAME_STATE_AT: usize = 0x100;
    let hour = fixture_game_hour_raw();

    let missing =
        CollectorStateFixture::new(&[p::DOMAIN57_CGAMESTATE_SINGLETON_RVA], 0x1000, 0x1000);
    let outcome = drive_compact_frame(
        Some(&missing.collector_state()),
        &compact_control_scheduling(&[RAW_SHIPS_DOMAIN]),
        hour,
        7,
    );
    assert_eq!(
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_skew_abs"),
        None,
        "an unreadable date root must publish NO skew value"
    );
    assert_eq!(
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_skew_missing"),
        Some(1)
    );

    for offset in [0i32, -3, 11] {
        let mut fixture =
            CollectorStateFixture::new(&[p::DOMAIN57_CGAMESTATE_SINGLETON_RVA], 0x1000, 0x1000);
        fixture
            .heap
            .write_i32(GAME_STATE_AT + GS_CURRENT_DATE_OFFSET, hour + offset);
        fixture.set_date_ptr_at(GAME_STATE_AT);
        let outcome = drive_compact_frame(
            Some(&fixture.collector_state()),
            &compact_control_scheduling(&[RAW_SHIPS_DOMAIN]),
            hour,
            7,
        );
        assert_eq!(
            outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_skew_abs"),
            Some(offset.unsigned_abs() as u64),
            "the frame must report the absolute distance to the staged date word"
        );
        assert_eq!(
            outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_skew_missing"),
            Some(0)
        );
    }
}

#[test]
fn the_fixtures_country_roster_decides_the_frames_no_scalar_source_bit() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    let staged_rvas = [p::DOMAIN57_CGAMESTATE_SINGLETON_RVA];
    let control = compact_control_scheduling(&[RAW_SHIPS_DOMAIN]);

    let without = CollectorStateFixture::new(&staged_rvas, 0x1000, 0x2000);
    let outcome = drive_compact_frame(
        Some(&without.collector_state()),
        &control,
        fixture_game_hour_raw(),
        7,
    );
    assert_ne!(
        outcome.timing.status_flags & COMPACT_STATUS_NO_SCALAR_SOURCE,
        0,
        "a not-attempted roster must badge the frame NO_SCALAR_SOURCE"
    );

    let mut with = CollectorStateFixture::new(&staged_rvas, 0x1000, 0x2000);
    let country_ptr = with.heap.at(0x100);
    with.set_verified_country_roster(
        "gamestate_country_table_verified",
        with.module.module_base() + p::DOMAIN57_COUNTRY_VTABLE_RVA,
        vec![test_country_row("POR", 1, country_ptr)],
    );
    let outcome = drive_compact_frame(
        Some(&with.collector_state()),
        &control,
        fixture_game_hour_raw(),
        7,
    );
    assert_eq!(
        outcome.timing.status_flags & COMPACT_STATUS_NO_SCALAR_SOURCE,
        0,
        "a verified, non-empty roster with a real vtable must clear NO_SCALAR_SOURCE"
    );
}

#[test]
fn a_driven_frame_records_each_workers_spawn_to_return_wall_and_outcome() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = compact_control_scheduling(&[RAW_SHIPS_DOMAIN, RAW_OOB_COMMAND_DOMAIN]);
    let outcome = drive_compact_frame(Some(&state), &control, fixture_game_hour_raw(), 7);

    for domain in [COMPACT_DOMAIN_SHIPS, COMPACT_DOMAIN_OOB_COMMAND] {
        let span = outcome
            .lane_span(domain)
            .unwrap_or_else(|| panic!("domain {domain} recorded a spawn -> return span"));
        assert!(
            span.end_us >= span.start_us,
            "domain {domain} span {span:?} runs backwards"
        );
        assert!(
            span.end_us <= outcome.timing.frame_time_us,
            "domain {domain} returned after the frame ended: {span:?} vs frame {}us",
            outcome.timing.frame_time_us
        );
        let perf = outcome
            .perf(domain)
            .unwrap_or_else(|| panic!("domain {domain} joined and emitted its perf record"));
        assert_eq!(
            perf.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            0,
            "domain {domain} returned a SUCCESSFUL read, not just a record: status {:#010x}",
            perf.status_flags
        );
        assert!(
            outcome.domain_data(domain).is_some(),
            "domain {domain} joined and emitted its data record"
        );
    }

    let worker_span_us = outcome
        .diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_worker_span_us")
        .expect("the lane-wall probe publishes the worker-phase span");
    let last_return_us = outcome
        .timing
        .lane_spans
        .iter()
        .map(|span| span.end_us)
        .max()
        .expect("at least one worker ran");
    assert!(
        last_return_us <= worker_span_us,
        "a worker returned at {last_return_us}us, after the published worker          span of {worker_span_us}us"
    );
}

/// The scope marks of a driven frame, read back from the lane-wall probe with
/// the spans they are measured against (lane PT).
struct ScopeMarks {
    readers_done_us: u64,
    scope_joined_us: u64,
    span_us: u64,
    /// `(lw_pub_start_us, lw_pub_end_us)`; absent when the frame ran no
    /// publisher.
    publication: Option<(u64, u64)>,
    /// The last GATED reader's return. The main lane's spans are the reads
    /// after the scope, not readers.
    last_reader_return_us: u64,
    /// d54's read: on the main thread, after the scope joined and before the
    /// span closed.
    post_scope_read: LaneSpan,
}

/// Ships and the OOB command tree as readers, d54 as the main-thread read that
/// follows the scope (its dependencies d27/d28 join the fan-out).
fn scope_marks_control() -> CollectorControl {
    compact_control_scheduling(&[
        RAW_SHIPS_DOMAIN,
        RAW_OOB_COMMAND_DOMAIN,
        RAW_ARMY_BATTLEPLANS_DEPTH_LIVE_DOMAIN,
    ])
}

fn scope_marks(outcome: &FrameOutcome) -> ScopeMarks {
    let unit = |name: &str| {
        outcome
            .diagnostic_unit(RC_PROBE_DOMAIN_ID, name)
            .unwrap_or_else(|| panic!("the lane-wall probe publishes {name}"))
    };
    let publication = match (
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_pub_start_us"),
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_pub_end_us"),
    ) {
        (Some(start), Some(end)) => Some((start, end)),
        (None, None) => None,
        (start, end) => panic!("the publication offsets come as a pair: {start:?}..{end:?}"),
    };
    let post_scope_read = *outcome
        .lane_span(COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE)
        .expect("d54 reads on the main thread after the scope");
    assert_eq!(
        post_scope_read.lane, LW_MAIN_LANE,
        "d54 is a main-thread read, not a gated worker"
    );
    ScopeMarks {
        readers_done_us: unit("lw_readers_done_us"),
        scope_joined_us: unit("lw_scope_joined_us"),
        span_us: unit("lw_worker_span_us"),
        publication,
        last_reader_return_us: outcome
            .timing
            .lane_spans
            .iter()
            .filter(|span| span.lane != LW_MAIN_LANE)
            .map(|span| span.end_us)
            .max()
            .expect("at least one reader ran"),
        post_scope_read,
    }
}

/// What every driven frame's marks obey, publisher or not: the readers return,
/// their close is stamped, the scope joins, the main-thread reads run, the span
/// closes. Each stamp is pinned by what it must follow and by what must follow
/// it, so neither can be a zero or the span close (Codex rounds 3 and 4). The
/// stamps' placement on the scope's own boundary is a source pin
/// (lane_wall_tests::the_scope_stamps_sit_on_the_scope_boundary_and_feed_the_wire_directly):
/// a duration bound across a thread exit and a join is not a law under load.
fn assert_scope_marks_bracket_the_join(marks: &ScopeMarks) {
    let ScopeMarks {
        readers_done_us,
        scope_joined_us,
        span_us,
        last_reader_return_us,
        post_scope_read,
        ..
    } = *marks;
    assert!(
        0 < last_reader_return_us && last_reader_return_us <= readers_done_us,
        "the readers' close ({readers_done_us}us) follows the last reader's return \
         ({last_reader_return_us}us)"
    );
    assert!(
        readers_done_us <= scope_joined_us,
        "the scope join ({scope_joined_us}us) follows the readers' close ({readers_done_us}us)"
    );
    // d54's read is a main-lane span on the same origin, after the scope
    // joined and before the span closed. A join stamped at the span close --
    // or anywhere after this read began -- puts the read before the join,
    // which no ordering of the code allows.
    assert!(
        scope_joined_us <= post_scope_read.start_us && post_scope_read.end_us <= span_us,
        "the post-scope main-thread read ({}..{}us) lies between the scope join \
         ({scope_joined_us}us) and the span close ({span_us}us)",
        post_scope_read.start_us,
        post_scope_read.end_us
    );
    // The main-thread reads after the scope are NOT the wait (Codex round 2);
    // the fixture's are well under 10 ms.
    assert!(
        span_us - scope_joined_us < 10_000,
        "the post-scope main-thread reads ({}us) are outside the wait",
        span_us - scope_joined_us
    );
}

/// The OS orders the publisher's thread against the readers'; the fixture
/// cannot. So a test that needs one ordering drives frames until the scheduler
/// produces it -- the ordering is the SHAPE under test, and the law is then
/// asserted on that frame, unconditionally. Under 20 spinning hogs on 16 cores
/// the readers of one frame took up to 385 ms and an instant publisher's first
/// slice came 6 ms late; a single frame with a fixed publisher length is not a
/// law there, eight attempts are.
const SHAPE_ATTEMPTS: usize = 8;

/// One driven frame with a publisher that sleeps `publisher_ms` (0 = returns at
/// once) and then sets a flag; panics if the publisher did not run.
fn drive_frame_with_publisher_ms(
    state: &CollectorState,
    control: &CollectorControl,
    publisher_ms: u64,
) -> FrameOutcome {
    let published = std::sync::atomic::AtomicBool::new(false);
    let outcome = drive_compact_frame_with_deferred_work(
        Some(state),
        control,
        fixture_game_hour_raw(),
        7,
        Some(Box::new(|| {
            if publisher_ms > 0 {
                std::thread::sleep(Duration::from_millis(publisher_ms));
            }
            published.store(true, Ordering::Release);
        })),
    );
    assert!(
        published.load(Ordering::Acquire),
        "the publisher runs inside the build, beside the readers"
    );
    outcome
}

/// One driven frame under its OWN owned timer session: the owner is begun
/// with the session's own origin as its edge, the frame is driven, the owner
/// is closed, and the session's report is read back. What the frame's service
/// looked like to the timer, beside what the frame published.
struct TimedFrame {
    outcome: FrameOutcome,
    /// What `exclude_wait` took out during this frame.
    excluded_ns: u64,
    /// The owner's observed service, every stage, as the report carries it.
    observed_service_ns: u64,
    /// The report's edge -> close, both on the timer's own instants: the edge
    /// IS the session's origin, so everything the timer attributed to the
    /// owner (the setup claimed at begin, every stage, the close) lies inside
    /// it with no slack for this test's clock. Observed service + excluded
    /// wait can never exceed it; a wait charged as service does (Codex rounds
    /// 6 and 7).
    edge_to_completion_ns: u64,
}

fn drive_timed_frame(
    state: &CollectorState,
    control: &CollectorControl,
    publisher_ms: Option<u64>,
    ordinal: u64,
) -> TimedFrame {
    let session = hourly_service_capture::start();
    let edge = hourly_service_capture::session_origin_for_tests().expect("the session is open");
    let owner = hourly_service_capture::ServiceOwner {
        logical_stream_id: 1,
        epoch: 1,
        game_hour_raw: fixture_game_hour_raw(),
        observation_ordinal: ordinal,
    };
    hourly_service_capture::begin(owner, Some(edge));
    let outcome = match publisher_ms {
        Some(ms) => drive_frame_with_publisher_ms(state, control, ms),
        None => drive_compact_frame(Some(state), control, fixture_game_hour_raw(), 7),
    };
    hourly_service_capture::close(owner, hourly_service_capture::RequestOutcome::Complete, None);
    let excluded_ns = hourly_service_capture::excluded_wait_ns_for_tests();
    let report =
        hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).expect("the session reports");
    drop(session);
    let sample = report
        .samples
        .iter()
        .find(|sample| sample.owner.observation_ordinal == ordinal)
        .expect("the owner's sample");
    TimedFrame {
        outcome,
        excluded_ns,
        observed_service_ns: sample
            .observed_service_ns
            .expect("a closed owner has an observed service"),
        edge_to_completion_ns: sample
            .edge_to_completion_ns
            .expect("the owner was begun with an edge"),
    }
}

/// The wait the timer excluded for ONE driven frame, checked against the
/// marks the same frame published and against the timer's own report: the
/// timer excludes exactly the join interval when the publisher outlived the
/// readers' close and nothing otherwise, and what it excluded is OUT of the
/// owner's observed service -- observed + excluded fits inside edge -> close,
/// which a wait charged as service overflows (Codex rounds 5 and 6).
fn assert_excluded_wait_matches_the_marks(frame: &TimedFrame, marks: &ScopeMarks) {
    let excluded_ns = frame.excluded_ns;
    assert!(
        frame.observed_service_ns + excluded_ns <= frame.edge_to_completion_ns,
        "observed service ({}ns) + excluded wait ({excluded_ns}ns) exceeds the owner's edge -> \
         close ({}ns): the wait was charged as service",
        frame.observed_service_ns,
        frame.edge_to_completion_ns
    );
    let waited_us = marks.scope_joined_us - marks.readers_done_us;
    match marks.publication {
        Some((_, pub_end_us)) if pub_end_us > marks.readers_done_us => {
            let excluded_us = excluded_ns / 1_000;
            assert!(
                excluded_us.abs_diff(waited_us) <= 1,
                "the publisher outlived the readers' close, so the timer excluded the join \
                 interval: excluded {excluded_us}us, scope_joined - readers_done {waited_us}us"
            );
        }
        _ => assert_eq!(
            excluded_ns, 0,
            "no publisher was waited for, so nothing is excluded: the {waited_us}us between \
             the readers' close and the join is this thread's own cleanup, and it is service"
        ),
    }
}

/// A frame that ran no publisher: both scope marks are present and bracket
/// the join, no publication offsets are published, and nothing was waited for.
///
/// kills: either mark missing or zero when there is no publisher (Codex round
/// 3, finding 2); the span close emitted as the join on a quiet frame (Codex
/// round 4, finding 1); a wait between the readers' close and the join.
#[test]
fn a_frame_that_ran_no_publisher_stamps_the_scope_join_between_its_readers_and_its_main_thread_reads(
) {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let frame = drive_timed_frame(&state, &scope_marks_control(), None, 1);
    let marks = scope_marks(&frame.outcome);
    assert_eq!(
        marks.publication, None,
        "no publisher ran, so the publication offsets are absent"
    );
    assert_scope_marks_bracket_the_join(&marks);
    assert_excluded_wait_matches_the_marks(&frame, &marks);
    // With no publisher there is nothing to wait for: the scope returns to the
    // main thread as soon as its closure does.
    assert!(
        marks.scope_joined_us - marks.readers_done_us < 5_000,
        "no publisher, no wait: readers done {}us, scope joined {}us",
        marks.readers_done_us,
        marks.scope_joined_us
    );
}

/// A publisher that ended before the readers: the common shape, where the sink
/// finishes inside the fan-out. The scope has nothing left to join when the
/// readers close, so the wait is nil and the join still follows the close.
///
/// kills: the publication's end emitted as the join (Codex round 4, finding
/// 2: on this ordering that puts the join before the readers' close, and the
/// audit then counts reader time as post-scope reads); a wait between the
/// readers' close and the join.
#[test]
fn a_publisher_that_ended_before_the_readers_leaves_no_wait_between_their_close_and_the_join() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = scope_marks_control();
    // The publisher is spawned before the first reader and does nothing but
    // set a flag, so its end lands in the first moments of the fan-out while
    // the readers' close needs every reader spawned, run and joined. THE SHAPE:
    // the publisher's end precedes the readers' close. Every attempt, shaped
    // or not, excluded exactly what its own marks say.
    let mut shaped = None;
    let mut last = None;
    for attempt in 0..SHAPE_ATTEMPTS {
        let frame = drive_timed_frame(&state, &control, Some(0), attempt as u64 + 1);
        let marks = scope_marks(&frame.outcome);
        assert_excluded_wait_matches_the_marks(&frame, &marks);
        let (_, pub_end_us) = marks.publication.expect("the frame ran a publisher");
        if pub_end_us <= marks.readers_done_us {
            shaped = Some(marks);
            break;
        }
        last = Some((pub_end_us, marks.readers_done_us));
    }
    let marks = shaped.unwrap_or_else(|| {
        panic!(
            "in {SHAPE_ATTEMPTS} frames the instant publisher never ended before the readers' \
             close (last: publisher ended {}us, readers done {}us)",
            last.map_or(0, |(end, _)| end),
            last.map_or(0, |(_, done)| done)
        )
    });
    let (pub_start_us, pub_end_us) = marks.publication.expect("the frame ran a publisher");
    assert!(
        pub_start_us <= pub_end_us && pub_end_us <= marks.readers_done_us,
        "the instant publisher ({pub_start_us}..{pub_end_us}us) ended before the readers' \
         close ({}us)",
        marks.readers_done_us
    );
    assert_scope_marks_bracket_the_join(&marks);
    // A publisher that had already ended is not waited for: the scope returns
    // to the main thread as soon as its closure does.
    assert!(
        marks.scope_joined_us - marks.readers_done_us < 5_000,
        "no wait on a publisher that had ended: readers done {}us, scope joined {}us",
        marks.readers_done_us,
        marks.scope_joined_us
    );
}

/// A publisher that outlives the readers: the wait the audit derives,
/// `lw_scope_joined_us - lw_readers_done_us`, covers its overrun, and the join
/// follows its end by the thread exit alone.
///
/// kills: `lw_readers_done_us` stamped after the join (it would then equal
/// the join, and the publisher below could not be seen past the readers); the
/// readers' stamp taken before the last reader joined; the join stamped at the
/// span close, or after a wait inserted past the scope's return (Codex round
/// 3).
#[test]
fn a_publisher_that_outlived_the_readers_is_the_wait_between_their_close_and_the_join() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = scope_marks_control();
    // THE SHAPE: the publisher's end follows the readers' close by at least
    // half the publisher's length. The publisher doubles each attempt (30 ms,
    // 60, ...), so a fan-out stretched by load is outlived on a later frame.
    // Every attempt, shaped or not, excluded exactly what its own marks say.
    let mut shaped = None;
    let mut last = None;
    for attempt in 0..SHAPE_ATTEMPTS {
        let publisher_ms = 30u64 << attempt;
        let frame = drive_timed_frame(&state, &control, Some(publisher_ms), attempt as u64 + 1);
        let marks = scope_marks(&frame.outcome);
        assert_excluded_wait_matches_the_marks(&frame, &marks);
        let (_, pub_end_us) = marks.publication.expect("the frame ran a publisher");
        if pub_end_us >= marks.readers_done_us + publisher_ms * 1_000 / 2 {
            shaped = Some((publisher_ms, marks));
            break;
        }
        last = Some((publisher_ms, pub_end_us, marks.readers_done_us));
    }
    let (publisher_ms, marks) = shaped.unwrap_or_else(|| {
        panic!(
            "in {SHAPE_ATTEMPTS} frames the publisher never outlived the readers' close by \
             half its length (last: {}ms publisher ended {}us, readers done {}us)",
            last.map_or(0, |(ms, _, _)| ms),
            last.map_or(0, |(_, end, _)| end),
            last.map_or(0, |(_, _, done)| done)
        )
    });
    let (pub_start_us, pub_end_us) = marks.publication.expect("the frame ran a publisher");
    assert!(
        pub_end_us.saturating_sub(pub_start_us) >= publisher_ms * 1_000,
        "the publisher ran {publisher_ms} ms: {pub_start_us}..{pub_end_us}"
    );
    assert!(
        pub_end_us >= marks.readers_done_us + publisher_ms * 1_000 / 2,
        "the publisher outlived the readers' close: readers done {}us, publisher ended \
         {pub_end_us}us -- a close stamped after the join cannot show this",
        marks.readers_done_us
    );
    assert_scope_marks_bracket_the_join(&marks);
    assert!(
        pub_end_us <= marks.scope_joined_us,
        "the scope joins after the publisher: publisher ended {pub_end_us}us, scope joined {}us",
        marks.scope_joined_us
    );
    let waited_us = marks.scope_joined_us - marks.readers_done_us;
    assert!(
        waited_us + 2_000 >= pub_end_us - marks.readers_done_us,
        "the wait between the readers' close and the join ({waited_us}us) covers the \
         publisher's overrun ({}us)",
        pub_end_us - marks.readers_done_us
    );
    // The publisher is the LAST thread in the scope here, so the join follows
    // its end by the thread's exit alone. A stamp taken later -- after a wait,
    // or after anything else the builder does past the scope -- inflates the
    // wait by that much. (The stamp's adjacency to the scope is pinned in
    // lane_wall_tests, where a wait too short for this bound is caught.)
    assert!(
        marks.scope_joined_us - pub_end_us < 10_000,
        "the scope join ({}us) follows the publisher's end ({pub_end_us}us) by the thread \
         exit alone, not by later work",
        marks.scope_joined_us
    );
}

const RAW_D47_DOMAIN: &str = "characters_leaders_cabinet_live";
const RAW_D82_DOMAIN: &str = "character_presentation_live";

fn d47_clock_fixture(hour: i32) -> CollectorStateFixture {
    use compact_contract::build_profile as p;
    const GAME_STATE: usize = 0x1000;
    let mut fixture =
        CollectorStateFixture::new(&[p::DOMAIN57_CGAMESTATE_SINGLETON_RVA], 0x1000, 0x4000);
    fixture.stage_scheduler_clock_at(GAME_STATE);
    fixture
        .heap
        .write_i32(GAME_STATE + GS_CURRENT_DATE_OFFSET, hour);
    fixture
}

fn d47_clock_verdict(outcome: &FrameOutcome) -> Option<u64> {
    outcome.diagnostic_unit(
        COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        "domain47_prewarm_clock_verdict",
    )
}

#[test]
fn the_frame_builder_decides_the_d47_prewarm_clock_verdict_against_its_own_label() {
    let hour = fixture_game_hour_raw();
    let control = compact_control_scheduling(&[RAW_D47_DOMAIN]);

    let fixture = d47_clock_fixture(hour);
    let in_hour = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    assert_eq!(
        d47_clock_verdict(&in_hour),
        Some(1),
        "the frame must report the prewarm's clock verdict, and an in-hour \
         prewarm is InFrameHour (1)"
    );
    assert_eq!(
        in_hour
            .trackers
            .characters_leaders_cabinet_live
            .roots
            .skipped_root_rows,
        0,
        "an in-hour prewarm must not poison its own roots"
    );

    let fixture = d47_clock_fixture(hour + 1);
    let wrong_hour = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    assert_eq!(
        d47_clock_verdict(&wrong_hour),
        None,
        "a refused domain's verdict unit must not be published under a label \
         its reads did not happen in"
    );
    assert_eq!(
        rejected_bracket_domains(
            &wrong_hour.timing.domain_clock_brackets,
            hour,
            HOUR_LABEL_MISMATCH_POLICY,
        )
        .get(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .map(|(reason, _, _)| *reason),
        Some(compact_contract::domain_gap::DomainGapReason::HourLabelMismatch),
        "the shipped policy must reject a wholly next-hour prewarm as an \
         HourLabelMismatch gap"
    );
    assert_eq!(
        wrong_hour
            .trackers
            .characters_leaders_cabinet_live
            .roots
            .skipped_root_rows,
        0,
        "a clock verdict must not poison the cache (the next frame's replay          decides it); a nonzero marker means the verdict or the reset touched it"
    );
    assert_eq!(
        wrong_hour
            .trackers
            .characters_leaders_cabinet_live
            .roots
            .stable_root_count(),
        0,
        "no root observed in the wrong hour may exist after the refusal"
    );
    assert!(
        rejected_bracket_domains(
            &wrong_hour.timing.domain_clock_brackets,
            hour,
            HourLabelMismatchPolicy::CountOnly,
        )
        .get(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .is_none(),
        "premise: nothing tore, so the refusal cannot have come from a bracket"
    );
}

#[test]
fn a_wrong_hour_or_unreadable_precheck_refuses_the_frame_and_keeps_the_cache() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let state = fixture.collector_state();
    let opening = |h: i32| {
        Some(ClockObservation {
            hour: h,
            ordinal: 1,
            at_us: 0,
        })
    };
    let cases: [(&str, Option<ClockObservation>, Domain47PrewarmClockVerdict); 2] = [
        (
            "WrongHour",
            opening(hour + 1),
            Domain47PrewarmClockVerdict::WrongHour,
        ),
        ("Unreadable", None, Domain47PrewarmClockVerdict::Unreadable),
    ];
    for (name, bracket_opening, expected) in cases {
        let mut cache = d47_cache_survives_refusal_tests::adopted_cache_with_witness();
        let witness_before = cache.witness.clone();
        let roots_before = cache.roots.clone();
        let (gen_before, epoch_before) = (cache.sweep_gen, cache.sweep_epoch);
        let backoff_before = cache.negative_prewarm_frames_remaining;
        assert!(backoff_before > 0, "fixture: a backoff window is armed");
        let observation = observe_compact_characters_leaders_cabinet_live(
            &state,
            0,
            &mut cache,
            hour,
            bracket_opening,
        );
        match observation {
            Domain47Observation::Rowless {
                status_flags,
                prewarm,
                ..
            } => {
                assert_eq!(
                    prewarm.clock_verdict, expected,
                    "{name}: the verdict is on the summary"
                );
                assert!(
                    !prewarm.ran,
                    "{name}: discovery must not run on a refused pre-check"
                );
                assert!(
                    !prewarm.negative_cache_hit,
                    "{name}: discovery's negative-backoff arm must not have been reached"
                );
                assert_eq!(
                    status_flags
                        & (COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA),
                    COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
                    "{name}: the refused frame carries the failure status A39/A36 gate on"
                );
                assert!(compact_contract::compact_status_is_failure(status_flags));
            }
            Domain47Observation::Built { .. } => {
                panic!("{name}: a refusing pre-check must not build an observation")
            }
        }
        assert_eq!(
            cache.witness, witness_before,
            "{name}: the witness must survive"
        );
        assert_eq!(
            cache.roots, roots_before,
            "{name}: the cache must be untouched"
        );
        assert_eq!(
            cache.roots.skipped_root_rows, 0,
            "{name}: no poison from a clock verdict"
        );
        assert_eq!(
            (cache.sweep_gen, cache.sweep_epoch),
            (gen_before, epoch_before)
        );
        assert_eq!(
            cache.negative_prewarm_frames_remaining, backoff_before,
            "{name}: the backoff word is untouched — discovery was never entered"
        );
        assert!(
            domain47_witness_reuse_eligible(
                cache.roots.has_stable_roots(),
                cache.roots.skipped_root_rows,
                cache.sweep_gen.is_some(),
                cache.witness.is_some(),
            ),
            "{name}: the next correctly-labelled frame may replay"
        );
    }

    let mut cache = d47_cache_survives_refusal_tests::adopted_cache_with_witness();
    let backoff_before = cache.negative_prewarm_frames_remaining;
    assert!(backoff_before > 0, "fixture: a backoff window is armed");
    let observation =
        observe_compact_characters_leaders_cabinet_live(&state, 0, &mut cache, hour, opening(hour));
    let prewarm = match observation {
        Domain47Observation::Rowless { prewarm, .. } => prewarm,
        Domain47Observation::Built { prewarm, .. } => prewarm,
    };
    assert_eq!(
        prewarm.clock_verdict,
        Domain47PrewarmClockVerdict::InFrameHour
    );
    assert!(
        prewarm.negative_cache_hit && cache.negative_prewarm_frames_remaining == backoff_before - 1,
        "control: an in-hour pre-check reaches discovery, which consumed a backoff frame here"
    );
}

#[test]
fn d47_records_exactly_one_bracket_and_it_is_the_workers_own() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_D47_DOMAIN]),
        hour,
        7,
    );

    let d47_brackets: Vec<&DomainClockBracket> = outcome
        .timing
        .domain_clock_brackets
        .iter()
        .filter(|bracket| bracket.tag == COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .collect();
    assert_eq!(
        d47_brackets.len(),
        1,
        "d47 records exactly one bracket per frame — the worker's, which \
         covers discovery and the frame walk alike"
    );
    for bracket in &d47_brackets {
        assert_eq!(
            bracket.opening.map(|observation| observation.hour),
            Some(hour),
            "the d47 bracket observed the fixture's staged hour"
        );
        assert_eq!(
            bracket.closing.map(|observation| observation.hour),
            Some(hour)
        );
        assert!(!bracket.worker_panicked);
    }
    assert!(
        rejected_bracket_domains(
            &outcome.timing.domain_clock_brackets,
            hour,
            HourLabelMismatchPolicy::Refuse,
        )
        .get(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .is_none(),
        "an in-hour frame must reject nothing, or the count above is of a \
         failure rather than an observation"
    );
}

#[test]
fn d47_reached_only_through_d82s_dependency_closure_still_brackets_its_prewarm() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_D82_DOMAIN]),
        hour,
        7,
    );

    assert!(
        outcome
            .perf(COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
            .is_some()
            || outcome
                .record(
                    COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
                    compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
                )
                .is_some(),
        "premise: d82's dependency closure emits d47 in this frame"
    );
    assert_eq!(
        d47_clock_verdict(&outcome),
        Some(1),
        "a dependency-reached d47 must still have its prewarm clock decided"
    );
    let d47_brackets = outcome
        .timing
        .domain_clock_brackets
        .iter()
        .filter(|bracket| bracket.tag == COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .count();
    assert_eq!(
        d47_brackets, 1,
        "a d47 reached through d82's dependency closure still spawns the \
         worker whose one bracket covers discovery (lane HT)"
    );
}

#[test]
fn the_worker_prechecks_then_discovers_then_walks() {
    const SOURCE: &str = include_str!("../../attach_log.rs");
    let lines: Vec<&str> = SOURCE.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with("fn observe_compact_characters_leaders_cabinet_live("))
        .expect("the observe half is in this file");
    let end = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|line| *line == "}")
            .expect("the observe half ends at a column-zero brace");
    let find = |needle: &str| -> usize {
        lines[start..end]
            .iter()
            .position(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && trimmed.contains(needle)
            })
            .map(|offset| start + offset + 1)
            .unwrap_or_else(|| panic!("the observe half contains `{needle}`"))
    };
    let precheck = find("domain47_worker_clock_precheck(");
    let discovery = find("prewarm_compact_characters_leaders_cabinet_roots(");
    let poisoned_gate = find("if cache.roots.skipped_root_rows != 0 {");
    let country = find("domain47_country_politics_roots(");
    let walk = find("observe_native_scheduled_frame_reusing(");
    assert!(
        precheck < discovery,
        "the opening-hour comparison must come BEFORE discovery, or a frame \
         it refuses pays the whole walk first (precheck at line {precheck}, \
         discovery at line {discovery})"
    );
    assert!(
        discovery < poisoned_gate,
        "the poisoned-root gate judges THIS frame's discovery, so it sits \
         after it (discovery at line {discovery}, gate at line \
         {poisoned_gate})"
    );
    assert!(
        poisoned_gate < country && country < walk,
        "the poisoned-root gate must precede the country-politics read and \
         the frame walk (GT residual #4): their product would be discarded by \
         the same gate downstream, so a pre-decided frame must not pay for \
         either (gate at line {poisoned_gate}, country at line {country}, \
         walk at line {walk})"
    );
}

#[test]
fn the_frame_probe_reports_how_many_brackets_missed_the_frames_label() {
    let hour = fixture_game_hour_raw();
    let control = compact_control_scheduling(&[RAW_D47_DOMAIN]);

    let fixture = d47_clock_fixture(hour);
    let in_hour = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    let held = in_hour
        .diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_bracketed_domains")
        .expect("the frame probe must report how many brackets were held");
    assert!(
        held > 0,
        "premise: this frame holds brackets, or the mismatch count below is \
         zero for an uninteresting reason"
    );
    assert_eq!(
        in_hour.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_mismatch_domains"),
        Some(0),
        "a frame whose readers ran in its own hour must report no mismatch"
    );

    let fixture = d47_clock_fixture(hour + 1);
    let wrong_hour = drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7);
    assert_eq!(
        wrong_hour.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_bracketed_domains"),
        Some(held),
        "premise: the two frames hold the same brackets, so the only thing \
         that differs below is the hour they read"
    );
    assert_eq!(
        wrong_hour.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_mismatch_domains"),
        Some(held),
        "every bracket in this frame read an hour that is not the label"
    );
    assert_eq!(
        wrong_hour.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_skew_abs"),
        Some(1),
        "and the frame's own start-of-sweep skew agrees, independently"
    );
    for (name, outcome) in [("in-hour", &in_hour), ("wrong-hour", &wrong_hour)] {
        assert_eq!(
            outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_unusable"),
            Some(0),
            "a frame whose label the calendar admits must say the rule applied ({name})"
        );
    }
}

#[test]
fn a_frame_whose_label_the_calendar_refuses_says_the_rule_did_not_apply() {
    const NO_CLOCK_SENTINEL: i32 = 0;
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_D47_DOMAIN]),
        NO_CLOCK_SENTINEL,
        7,
    );

    assert_eq!(
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_unusable"),
        Some(1),
        "a frame the rule cannot be applied to must say so"
    );
    assert_eq!(
        outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_mismatch_domains"),
        Some(0),
        "and it must not also report a mismatch it was in no position to find"
    );
}

mod d43_at {
    pub(super) const GAME_STATE: usize = 0x0_0000;
    pub(super) const MANAGER: usize = 0x0_1000;
    pub(super) const OUTER_TABLE_0: usize = 0x0_2000;
    pub(super) const OUTER_TABLE_1: usize = 0x0_2100;
    pub(super) const INNER_BODY_0: usize = 0x0_3000;
    pub(super) const INNER_BODY_1: usize = 0x0_3100;
    pub(super) const COUNTRY_1: usize = 0x0_4000;
    pub(super) const COUNTRY_2: usize = 0x0_6000;
    pub(super) const MARKET_STATUS_1: usize = 0x0_8000;
    pub(super) const MARKET_STATUS_2: usize = 0x0_8100;
    pub(super) const CONTRACT_1: usize = 0x0_9000;
    pub(super) const CONTRACT_2: usize = 0x0_9400;
    pub(super) const EQUIPMENT_SUBBUF_1: usize = 0x0_A000;
    pub(super) const DELIVERY_SUBBUF_1: usize = 0x0_A100;
    pub(super) const EQUIPMENT_SUBBUF_2: usize = 0x0_A200;
    pub(super) const DELIVERY_SUBBUF_2: usize = 0x0_A300;
    pub(super) const VARIANT_EQUIPMENT: usize = 0x0_B000;
    pub(super) const VARIANT_DELIVERY: usize = 0x0_B100;
    pub(super) const LINK_TABLE: usize = 0x0_C000;
    pub(super) const HEAP_LEN: usize = 0x1_0000;
    pub(super) const LINK_COUNT: i32 = 16;
    pub(super) const COUNTRY_EXTENT: i32 = 3;

    pub(super) const OUTER_COUNT: i32 = 3;
    pub(super) const CONTRACT_1_ID: u32 = 7001;
    pub(super) const CONTRACT_2_ID: u32 = 7002;
    pub(super) const EQUIPMENT_VARIANT_ID: u32 = 4242;
    pub(super) const DELIVERY_AMOUNT_RAW: i32 = 250_000;
    pub(super) const DELIVERY_VARIANT_ID: u32 = 4243;
}

fn market_trade_frame_fixture() -> CollectorStateFixture {
    use compact_contract::build_profile as p;
    use d43_at as at;

    let mut fixture = CollectorStateFixture::new(
        &[
            C_GAME_STATE_DAILY_UPDATE_OFFSET,
            GET_LONG_STRING_OFFSET,
            p::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        ],
        0x1000,
        at::HEAP_LEN,
    );
    fixture.stage_profile_sentinels();
    fixture.stage_scheduler_clock_at(at::GAME_STATE);

    let base = fixture.module.module_base();
    let country_vtable = base + p::DOMAIN57_COUNTRY_VTABLE_RVA;
    let contract_vtable = base + p::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    let variant_vtable = base + VARIANT_VTABLE_RVA;

    let manager = fixture.heap.at(at::MANAGER);
    fixture.heap.write_u64(
        at::GAME_STATE + CGAMESTATE_MARKET_MANAGER_OFF,
        manager as u64,
    );

    // Both staged countries are SELF-CANONICAL here -- no dynamic release has
    fixture.heap.write_u64(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_PTR,
        fixture.heap.at(at::LINK_TABLE) as u64,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_COUNT,
        at::LINK_COUNT,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + CGAMESTATE_COUNTRY_COUNT,
        at::COUNTRY_EXTENT,
    );
    fixture.heap.write_i32(at::LINK_TABLE + 4, 1);
    fixture.heap.write_i32(at::LINK_TABLE + 8, 2);

    for (country_at, status_at) in [
        (at::COUNTRY_1, at::MARKET_STATUS_1),
        (at::COUNTRY_2, at::MARKET_STATUS_2),
    ] {
        fixture.heap.write_u64(country_at, country_vtable as u64);
        let status = fixture.heap.at(status_at);
        fixture.heap.write_u64(
            country_at + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF,
            status as u64,
        );
        fixture.heap.write_u64(
            status_at + CEQUIPMENT_MARKET_STATUS_MANAGER_OFF,
            manager as u64,
        );
    }
    let country_1 = fixture.heap.at(at::COUNTRY_1);
    let country_2 = fixture.heap.at(at::COUNTRY_2);
    fixture.set_verified_country_roster(
        "gamestate_country_table_verified",
        country_vtable,
        vec![
            test_country_row("SEL", 1, country_1),
            test_country_row("BUY", 2, country_2),
        ],
    );

    for (variant_at, variant_id) in [
        (at::VARIANT_EQUIPMENT, at::EQUIPMENT_VARIANT_ID),
        (at::VARIANT_DELIVERY, at::DELIVERY_VARIANT_ID),
    ] {
        fixture.heap.write_u64(variant_at, variant_vtable as u64);
        fixture
            .heap
            .write_i32(variant_at + VARIANT_DEF_TYPE, VARIANT_TYPE as i32);
        fixture
            .heap
            .write_i32(variant_at + VARIANT_DEF_ID, variant_id as i32);
    }
    for (subbuf_at, variant_at) in [
        (at::EQUIPMENT_SUBBUF_1, at::VARIANT_EQUIPMENT),
        (at::DELIVERY_SUBBUF_1, at::VARIANT_DELIVERY),
        (at::EQUIPMENT_SUBBUF_2, at::VARIANT_EQUIPMENT),
        (at::DELIVERY_SUBBUF_2, at::VARIANT_DELIVERY),
    ] {
        let variant = fixture.heap.at(variant_at);
        fixture.heap.write_u64(subbuf_at, variant as u64);
        fixture
            .heap
            .write_i32(subbuf_at + 8, at::DELIVERY_AMOUNT_RAW);
    }

    for (contract_at, contract_id, equipment_at, delivery_at) in [
        (
            at::CONTRACT_1,
            at::CONTRACT_1_ID,
            at::EQUIPMENT_SUBBUF_1,
            at::DELIVERY_SUBBUF_1,
        ),
        (
            at::CONTRACT_2,
            at::CONTRACT_2_ID,
            at::EQUIPMENT_SUBBUF_2,
            at::DELIVERY_SUBBUF_2,
        ),
    ] {
        fixture.heap.write_u64(contract_at, contract_vtable as u64);
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_ID_TYPE,
            p::DOMAIN43_PURCHASE_CONTRACT_TYPE as i32,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_ID_ID,
            contract_id as i32,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_SELLER_TAG_DB_ID,
            1,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_TAG_DB_ID,
            2,
        );
        fixture.heap.write_u64(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR,
            country_2 as u64,
        );
        let equipment = fixture.heap.at(equipment_at);
        let delivery = fixture.heap.at(delivery_at);
        fixture.heap.write_u64(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_EQUIPMENT_SUBBUF,
            equipment as u64,
        );
        fixture.heap.write_u64(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_DELIVERY_SUBBUF,
            delivery as u64,
        );
    }

    let contract_1 = fixture.heap.at(at::CONTRACT_1);
    let contract_2 = fixture.heap.at(at::CONTRACT_2);
    for body_at in [at::INNER_BODY_0, at::INNER_BODY_1] {
        fixture.heap.write_u64(body_at, contract_1 as u64);
        fixture.heap.write_u64(body_at + 8, contract_2 as u64);
    }
    let inner_body_0 = fixture.heap.at(at::INNER_BODY_0);
    let inner_body_1 = fixture.heap.at(at::INNER_BODY_1);

    let write_vector = |heap: &mut FixtureRegion, at: usize, begin: u64, count: i32| {
        heap.write_u64(at + MARKET_VECTOR_BEGIN_OFF, begin);
        heap.write_i32(at + MARKET_VECTOR_CAP_OFF, count);
        heap.write_i32(at + MARKET_VECTOR_COUNT_OFF, count);
    };
    write_vector(&mut fixture.heap, at::OUTER_TABLE_0, inner_body_0 as u64, 2);
    write_vector(
        &mut fixture.heap,
        at::OUTER_TABLE_0 + MARKET_CONTRACT_VECTOR_STRIDE,
        0,
        0,
    );
    write_vector(
        &mut fixture.heap,
        at::OUTER_TABLE_0 + 2 * MARKET_CONTRACT_VECTOR_STRIDE,
        0,
        0,
    );
    write_vector(&mut fixture.heap, at::OUTER_TABLE_1, 0, 0);
    write_vector(
        &mut fixture.heap,
        at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE,
        inner_body_1 as u64,
        2,
    );
    write_vector(
        &mut fixture.heap,
        at::OUTER_TABLE_1 + 2 * MARKET_CONTRACT_VECTOR_STRIDE,
        0,
        0,
    );

    let outer_0 = fixture.heap.at(at::OUTER_TABLE_0);
    let outer_1 = fixture.heap.at(at::OUTER_TABLE_1);
    write_vector(
        &mut fixture.heap,
        at::MANAGER + MARKET_CONTRACT_TABLE_OFFSETS[0],
        outer_0 as u64,
        at::OUTER_COUNT,
    );
    write_vector(
        &mut fixture.heap,
        at::MANAGER + MARKET_CONTRACT_TABLE_OFFSETS[1],
        outer_1 as u64,
        at::OUTER_COUNT,
    );

    fixture
}

#[test]
fn a_populated_market_world_makes_domain43_emit_purchase_contract_rows_offline() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let fixture = market_trade_frame_fixture();
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_eq!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "the staged market roots are readable, so the contract census must not \
         report SOURCE_UNAVAILABLE; status {:#010x}",
        perf.status_flags
    );
    assert_eq!(
        perf.row_count, 2,
        "both staged purchase contracts were observed (t21 measured 0)"
    );
    assert_eq!(perf.unit_count, 2, "and both were emitted");

    let state = fixture.collector_state();
    let country_by_tag = BTreeMap::from([
        (1u32, fixture.heap.at(d43_at::COUNTRY_1)),
        (2u32, fixture.heap.at(d43_at::COUNTRY_2)),
    ]);
    let vtable =
        state.base + compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    let contract_ptrs = compact_market_trade_rooted_contract_ptrs(&state, vtable, &country_by_tag)
        .expect("the staged census is admissible");
    assert_eq!(
        contract_ptrs.len(),
        2,
        "both staged contracts hold their seller AND buyer membership"
    );
    let rows = contract_ptrs
        .iter()
        .map(|contract_ptr| {
            read_compact_market_trade_row(state.base, *contract_ptr, vtable, &country_by_tag)
                .expect("a staged contract decodes")
        })
        .collect::<Vec<_>>();
    let staged_row = |contract_id: u32| CompactMarketTradeRow {
        contract_id,
        contract_type: compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_TYPE,
        flags: COMPACT_MARKET_TRADE_ROW_FLAG_BUYER_PTR_VALID
            | COMPACT_MARKET_TRADE_ROW_FLAG_EQUIPMENT_SUBBUF_VALID
            | COMPACT_MARKET_TRADE_ROW_FLAG_DELIVERY_SUBBUF_VALID
            | COMPACT_MARKET_TRADE_ROW_FLAG_VARIANT_VALID
            | COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID,
        seller_tag_db_id: 1,
        buyer_tag_db_id: 2,
        equipment_variant_id: d43_at::EQUIPMENT_VARIANT_ID,
        amount_raw: i64::from(d43_at::DELIVERY_AMOUNT_RAW),
        equipment_subbuf_status: COMPACT_MARKET_TRADE_SUBBUF_STATUS_VALID,
        delivery_subbuf_status: COMPACT_MARKET_TRADE_SUBBUF_STATUS_VALID,
        buyer_ptr_status: COMPACT_MARKET_TRADE_PTR_STATUS_VALID,
    };
    assert_eq!(
        rows,
        vec![
            staged_row(d43_at::CONTRACT_1_ID),
            staged_row(d43_at::CONTRACT_2_ID)
        ],
        "the production contract reader must return exactly the staged values"
    );

    let data = outcome
        .domain_data(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted a DOMAIN_DATA record");
    assert_eq!(
        le_u32(&data.payload, 8),
        2,
        "the payload header states both contract rows"
    );
    assert_eq!(
        le_u32(&data.payload, 24),
        2,
        "and both were observed rather than merely emitted"
    );
    let mut contract_ids = Vec::new();
    for index in 0..2usize {
        contract_ids.push(le_u32(&data.payload, 48 + index * 48));
    }
    assert_eq!(
        contract_ids,
        vec![d43_at::CONTRACT_1_ID, d43_at::CONTRACT_2_ID],
        "the payload's contract block carries the staged contract ids"
    );

    assert_ne!(
        perf.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "the market-only fixture leaves d43's other families absent"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(0),
        "the census succeeded, so it must name no failing stage -- the ONE value \
         that distinguishes a healthy census from the t21 wire"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_tag_db_id"
        ),
        Some(u64::from(u32::MAX)),
        "and no offending country"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_excluded_slots"
        ),
        Some(0),
        "a table wide enough for both database ids excludes neither"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_first_excluded_tag_db_id"
        ),
        Some(u64::from(u32::MAX)),
        "and names no excluded slot"
    );
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_MARKET_TRADE_LIVE);
}

/// 0x08` actually reads for a released country — 5 and 7, resolving to slots 1
/// MUTATION RUN (`--release`): restored `if tag < index_bound` in
#[test]
fn the_contract_census_bounds_the_link_value_not_the_census_key() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    const SEL_TAG_DB_ID: u32 = 5;
    const BUY_TAG_DB_ID: u32 = 7;

    let mut fixture = market_trade_frame_fixture();
    let country_1 = fixture.heap.at(d43_at::COUNTRY_1);
    let country_2 = fixture.heap.at(d43_at::COUNTRY_2);
    // Two RELEASED countries: their `CCountry + 0x08` values are link indices 5
    fixture
        .heap
        .write_i32(d43_at::LINK_TABLE + 4 * SEL_TAG_DB_ID as usize, 1);
    fixture
        .heap
        .write_i32(d43_at::LINK_TABLE + 4 * BUY_TAG_DB_ID as usize, 2);
    let country_vtable =
        fixture.module.module_base() + compact_contract::build_profile::DOMAIN57_COUNTRY_VTABLE_RVA;
    fixture.set_verified_country_roster(
        "gamestate_country_table_verified",
        country_vtable,
        vec![
            test_country_row("SEL", SEL_TAG_DB_ID, country_1),
            test_country_row("BUY", BUY_TAG_DB_ID, country_2),
        ],
    );

    let state = fixture.collector_state();
    let census = BTreeMap::from([(SEL_TAG_DB_ID, country_1), (BUY_TAG_DB_ID, country_2)]);
    assert!(
        d43_at::OUTER_COUNT < SEL_TAG_DB_ID as i32,
        "the census keys must be OUTSIDE the table width, or the raw comparison \
         would admit them too and this test would prove nothing"
    );
    let vtable =
        state.base + compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    let contract_ptrs = compact_market_trade_rooted_contract_ptrs(&state, vtable, &census);
    assert_eq!(
        contract_ptrs.as_deref().map_or(0, <[_]>::len),
        2,
        "both contracts are still admitted when the census is keyed by CCountry+0x08"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_EXCLUDED_SLOTS.load(Ordering::Relaxed),
        0,
        "and neither country is an excluded slot"
    );
}

/// MUTATION RUN (`--release`): replaced
#[test]
fn the_contract_census_refuses_when_the_link_vector_is_unreadable() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    let country_1 = fixture.heap.at(d43_at::COUNTRY_1);
    let country_2 = fixture.heap.at(d43_at::COUNTRY_2);
    fixture.heap.write_u64(
        d43_at::GAME_STATE
            + compact_contract::build_profile::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_PTR,
        0,
    );

    let state = fixture.collector_state();
    let census = BTreeMap::from([(1u32, country_1), (2u32, country_2)]);
    let vtable =
        state.base + compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    assert!(
        compact_market_trade_rooted_contract_ptrs(&state, vtable, &census).is_none(),
        "the census cannot say which countries the engine's tables index without \
         the link vector"
    );
}

#[test]
fn a_one_sided_purchase_contract_membership_names_the_stage_that_refused_it() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    fixture.heap.write_i32(
        d43_at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE + MARKET_VECTOR_COUNT_OFF,
        1,
    );
    fixture.heap.write_i32(
        d43_at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE + MARKET_VECTOR_CAP_OFF,
        1,
    );

    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_eq!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        "a refused census is source-unavailable, as in live capture t21"
    );
    assert_eq!(perf.row_count, 0, "and publishes no contract rows");
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(u64::from(domain43_contract_stage::MEMBERSHIP_INCOMPLETE)),
        "the incomplete-membership refusal must name its own stage"
    );

    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_scope_countries"
        ),
        Some(2),
        "the census states the country set it was handed"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_snapshot_countries"
        ),
        Some(2),
        "and the active-country subset it was widened from"
    );
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_outer_count"),
        Some(u64::from(d43_at::OUTER_COUNT as u32)),
        "and the engine's own outer table width"
    );
}

#[test]
fn a_country_without_a_market_status_wrapper_refuses_the_whole_contract_census() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    fixture
        .heap
        .write_u64(d43_at::COUNTRY_2 + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF, 0);

    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_eq!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        "one country without a wrapper voids the whole census"
    );
    assert_eq!(perf.row_count, 0, "so no contract row is published at all");
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(u64::from(
            domain43_contract_stage::CORROBORATE_WRAPPER_NOT_HEAP
        )),
        "the refusal names the corroboration stage, not one of the other nineteen"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_tag_db_id"
        ),
        Some(2),
        "the offending country accompanies the stage"
    );
}

mod d43_t22_at {
    pub(super) const SNAPSHOT_COUNTRIES: u32 = 439;
    pub(super) const OUTER_COUNT: i32 = 440;
    pub(super) const DYNAMIC_SLOTS: u32 = 3;
    pub(super) const SCOPE_COUNTRIES: u32 = SNAPSHOT_COUNTRIES + DYNAMIC_SLOTS;

    pub(super) const GAME_STATE: usize = 0x0_0000;
    pub(super) const MANAGER: usize = 0x0_1000;
    pub(super) const OUTER_TABLE_0: usize = 0x0_2000;
    pub(super) const OUTER_TABLE_1: usize = 0x0_5000;
    pub(super) const INNER_BODY_0: usize = 0x0_8000;
    pub(super) const INNER_BODY_1: usize = 0x0_8100;
    pub(super) const CONTRACT_1: usize = 0x0_9000;
    pub(super) const CONTRACT_2: usize = 0x0_9400;
    pub(super) const STATUS_BASE: usize = 0x1_0000;
    pub(super) const STATUS_STRIDE: usize = 0x80;
    pub(super) const COUNTRY_BASE: usize = 0x2_0000;
    pub(super) const COUNTRY_STRIDE: usize = 0x1000;
    pub(super) const LINK_TABLE: usize = 0x1D_B000;
    pub(super) const HEAP_LEN: usize = 0x1E_0000;

    pub(super) const CONTRACT_1_ID: u32 = 8001;
    pub(super) const CONTRACT_2_ID: u32 = 8002;

    pub(super) fn country_at(tag_db_id: u32) -> usize {
        COUNTRY_BASE + (tag_db_id as usize - 1) * COUNTRY_STRIDE
    }

    pub(super) fn status_at(tag_db_id: u32) -> usize {
        STATUS_BASE + (tag_db_id as usize - 1) * STATUS_STRIDE
    }
}

fn market_trade_t22_shape_fixture() -> CollectorStateFixture {
    use compact_contract::build_profile as p;
    use d43_t22_at as at;

    let mut fixture = CollectorStateFixture::new(
        &[
            C_GAME_STATE_DAILY_UPDATE_OFFSET,
            GET_LONG_STRING_OFFSET,
            p::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        ],
        0x1000,
        at::HEAP_LEN,
    );
    fixture.stage_profile_sentinels();
    fixture.stage_scheduler_clock_at(at::GAME_STATE);

    let base = fixture.module.module_base();
    let country_vtable = base + p::DOMAIN57_COUNTRY_VTABLE_RVA;
    let contract_vtable = base + p::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;

    let manager = fixture.heap.at(at::MANAGER);
    fixture.heap.write_u64(
        at::GAME_STATE + CGAMESTATE_MARKET_MANAGER_OFF,
        manager as u64,
    );

    // no dynamic release had happened -- so each `CCountry + 0x08` maps to its
    fixture.heap.write_u64(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_PTR,
        fixture.heap.at(at::LINK_TABLE) as u64,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_COUNT,
        at::SCOPE_COUNTRIES as i32 + 1,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + CGAMESTATE_COUNTRY_COUNT,
        at::SCOPE_COUNTRIES as i32 + 1,
    );

    for tag_db_id in 1..=at::SCOPE_COUNTRIES {
        let country_at = at::country_at(tag_db_id);
        let status_at = at::status_at(tag_db_id);
        fixture
            .heap
            .write_i32(at::LINK_TABLE + tag_db_id as usize * 4, tag_db_id as i32);
        fixture.heap.write_u64(country_at, country_vtable as u64);
        let status = fixture.heap.at(status_at);
        fixture.heap.write_u64(
            country_at + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF,
            status as u64,
        );
        fixture.heap.write_u64(
            status_at + CEQUIPMENT_MARKET_STATUS_MANAGER_OFF,
            manager as u64,
        );
    }

    let country_2 = fixture.heap.at(at::country_at(2));
    for (contract_at, contract_id) in [
        (at::CONTRACT_1, at::CONTRACT_1_ID),
        (at::CONTRACT_2, at::CONTRACT_2_ID),
    ] {
        fixture.heap.write_u64(contract_at, contract_vtable as u64);
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_ID_TYPE,
            p::DOMAIN43_PURCHASE_CONTRACT_TYPE as i32,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_ID_ID,
            contract_id as i32,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_SELLER_TAG_DB_ID,
            1,
        );
        fixture.heap.write_i32(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_TAG_DB_ID,
            2,
        );
        fixture.heap.write_u64(
            contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR,
            country_2 as u64,
        );
    }
    let contract_1 = fixture.heap.at(at::CONTRACT_1);
    let contract_2 = fixture.heap.at(at::CONTRACT_2);
    for body_at in [at::INNER_BODY_0, at::INNER_BODY_1] {
        fixture.heap.write_u64(body_at, contract_1 as u64);
        fixture.heap.write_u64(body_at + 8, contract_2 as u64);
    }

    let write_vector = |heap: &mut FixtureRegion, at: usize, begin: u64, count: i32| {
        heap.write_u64(at + MARKET_VECTOR_BEGIN_OFF, begin);
        heap.write_i32(at + MARKET_VECTOR_CAP_OFF, count);
        heap.write_i32(at + MARKET_VECTOR_COUNT_OFF, count);
    };
    for (table_at, occupied_slot, body_at) in [
        (at::OUTER_TABLE_0, 1usize, at::INNER_BODY_0),
        (at::OUTER_TABLE_1, 2usize, at::INNER_BODY_1),
    ] {
        let body = fixture.heap.at(body_at);
        for slot in 0..at::OUTER_COUNT as usize {
            let (begin, count) = if slot == occupied_slot {
                (body as u64, 2)
            } else {
                (0, 0)
            };
            write_vector(
                &mut fixture.heap,
                table_at + slot * MARKET_CONTRACT_VECTOR_STRIDE,
                begin,
                count,
            );
        }
    }
    let outer_0 = fixture.heap.at(at::OUTER_TABLE_0);
    let outer_1 = fixture.heap.at(at::OUTER_TABLE_1);
    write_vector(
        &mut fixture.heap,
        at::MANAGER + MARKET_CONTRACT_TABLE_OFFSETS[0],
        outer_0 as u64,
        at::OUTER_COUNT,
    );
    write_vector(
        &mut fixture.heap,
        at::MANAGER + MARKET_CONTRACT_TABLE_OFFSETS[1],
        outer_1 as u64,
        at::OUTER_COUNT,
    );

    fixture
}

#[test]
fn the_t22_contract_census_shape_emits_rows_with_named_exclusions() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let fixture = market_trade_t22_shape_fixture();
    let state = fixture.collector_state();
    let vtable =
        state.base + compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;

    let contract_country_ptr_by_tag_id = (1..=d43_t22_at::SCOPE_COUNTRIES)
        .map(|tag_db_id| {
            (
                tag_db_id,
                fixture.heap.at(d43_t22_at::country_at(tag_db_id)),
            )
        })
        .collect::<BTreeMap<u32, usize>>();
    assert_eq!(
        contract_country_ptr_by_tag_id.len(),
        d43_t22_at::SCOPE_COUNTRIES as usize,
        "the staged scope is the width t22 measured"
    );

    let contract_ptrs =
        compact_market_trade_rooted_contract_ptrs(&state, vtable, &contract_country_ptr_by_tag_id)
            .expect(
                "a scope wider than the engine's outer table must no longer void the family -- \
                 this is the t22 refusal (stage 303) itself",
            );
    assert_eq!(
        contract_ptrs.len(),
        2,
        "both staged contracts are collected from the covered slots"
    );

    let rows = contract_ptrs
        .iter()
        .map(|contract_ptr| {
            read_compact_market_trade_row(
                state.base,
                *contract_ptr,
                vtable,
                &contract_country_ptr_by_tag_id,
            )
            .expect("a staged contract decodes")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter().map(|row| row.contract_id).collect::<Vec<_>>(),
        vec![d43_t22_at::CONTRACT_1_ID, d43_t22_at::CONTRACT_2_ID],
        "the rows carry the staged contract ids"
    );
    assert!(
        rows.iter()
            .all(|row| row.seller_tag_db_id == 1 && row.buyer_tag_db_id == 2),
        "and the staged counterparties"
    );

    assert_eq!(
        DOMAIN43_CONTRACT_SCOPE_COUNTRIES.load(Ordering::Relaxed),
        d43_t22_at::SCOPE_COUNTRIES,
        "the census still states the country set it was handed"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_OUTER_COUNT.load(Ordering::Relaxed),
        d43_t22_at::OUTER_COUNT as u32,
        "and the engine's own narrowest table width"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_FAILURE_STAGE.load(Ordering::Relaxed),
        0,
        "the census succeeded, so it names no failing stage (t22 named 303)"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_EXCLUDED_SLOTS.load(Ordering::Relaxed),
        d43_t22_at::DYNAMIC_SLOTS,
        "the reserved dynamic slots past the engine's table are excluded, not refused"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_FIRST_EXCLUDED_TAG_DB_ID.load(Ordering::Relaxed),
        d43_t22_at::OUTER_COUNT as u32,
        "and are named: the first is the lowest database id the table cannot index"
    );
    assert_eq!(
        DOMAIN43_CONTRACT_LAST_EXCLUDED_TAG_DB_ID.load(Ordering::Relaxed),
        d43_t22_at::SCOPE_COUNTRIES,
        "through the highest slot in scope"
    );
}

#[test]
fn a_slot_outside_the_engine_table_is_excluded_and_named_rather_than_refused() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    for table_at in [d43_at::OUTER_TABLE_0, d43_at::OUTER_TABLE_1] {
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_COUNT_OFF,
            2,
        );
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_CAP_OFF,
            2,
        );
    }

    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    // The walk is unchanged -- the other 440 slots' contracts still reach the wire; only the word changed, because a contract owned by an excluded country is never a candidate and the census's own words would read exact over a short set.
    assert_eq!(
        perf.status_flags & (COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA),
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        "an excluded slot is a scope the census could not reach; status {:#010x}",
        perf.status_flags
    );
    assert_eq!(
        perf.row_count, 2,
        "the contracts in the covered slots still reach the wire"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(0),
        "and no guard refused the family"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_excluded_slots"
        ),
        Some(1),
        "the one database id the tables cannot index is excluded"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_first_excluded_tag_db_id"
        ),
        Some(2),
        "and named"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_last_excluded_tag_db_id"
        ),
        Some(2),
        "at both ends of a one-slot range"
    );
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_outer_count"),
        Some(2),
        "and the width evidence still states the bound that applied"
    );
}

#[test]
fn an_excluded_slot_without_a_market_status_wrapper_no_longer_voids_the_census() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    for table_at in [d43_at::OUTER_TABLE_0, d43_at::OUTER_TABLE_1] {
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_COUNT_OFF,
            2,
        );
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_CAP_OFF,
            2,
        );
    }
    fixture
        .heap
        .write_u64(d43_at::COUNTRY_2 + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF, 0);

    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_eq!(
        perf.row_count, 2,
        "the covered countries' contracts survive a wrapper-less excluded slot"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(0),
        "the corroboration loop never reaches a slot the engine's table cannot index"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_excluded_slots"
        ),
        Some(1),
        "and the slot is accounted for rather than silently skipped"
    );
}

#[test]
fn a_table_that_can_index_no_scope_country_still_refuses_the_census() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    for table_at in [d43_at::OUTER_TABLE_0, d43_at::OUTER_TABLE_1] {
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_COUNT_OFF,
            1,
        );
        fixture.heap.write_i32(
            d43_at::MANAGER + table_at_offset(table_at) + MARKET_VECTOR_CAP_OFF,
            1,
        );
    }

    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_eq!(
        perf.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        "a table that covers nothing is a refused census, not an empty one"
    );
    assert_eq!(perf.row_count, 0, "and publishes no contract rows");
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_stage"
        ),
        Some(u64::from(
            domain43_contract_stage::OUTER_NARROWER_THAN_SCOPE
        )),
        "and it is stage 303 that says so"
    );
}

fn table_at_offset(table_at: usize) -> usize {
    match table_at {
        d43_at::OUTER_TABLE_0 => MARKET_CONTRACT_TABLE_OFFSETS[0],
        d43_at::OUTER_TABLE_1 => MARKET_CONTRACT_TABLE_OFFSETS[1],
        other => panic!("{other:#x} is not a staged d43 outer table"),
    }
}

const AO_MOVED_DOMAINS: [(&str, u16, &str); 4] = [
    (
        "d85",
        COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
        RAW_SELECTED_DESIGNER_IDENTITY_LIVE_DOMAIN,
    ),
    (
        "d94",
        COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE,
        RAW_POV_PROVINCE_VISIBILITY_LIVE_DOMAIN,
    ),
    (
        "d95",
        COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE,
        RAW_POV_SPOTTED_UNITS_LIVE_DOMAIN,
    ),
    (
        "d97",
        COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE,
        RAW_PEACE_CONFERENCE_LIVE_DOMAIN,
    ),
];

#[test]
fn the_four_moved_reads_run_between_the_prewarm_and_the_worker_fanout() {
    const SOURCE: &str = include_str!("../../attach_log.rs");
    let lines: Vec<&str> = SOURCE.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with("fn build_compact_extraction_gate_records("))
        .expect("the gate function is in this file");
    let end = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|line| *line == "}")
            .expect("the gate function ends at a column-zero brace");
    let find = |needle: &str| -> usize {
        lines[start..end]
            .iter()
            .position(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && trimmed.contains(needle)
            })
            .map(|offset| start + offset + 1)
            .unwrap_or_else(|| panic!("the gate function contains `{needle}`"))
    };

    let tracker = find("let lane_tracker = FrameLaneTracker::new(");
    let fanout = find("thread::scope(|scope| {");
    assert!(
        tracker < fanout,
        "premise: the lane tracker precedes the fan-out (tracker at line \
         {tracker}, fan-out at line {fanout})"
    );

    for (domain, needle) in [
        (
            "d85",
            "let selected_designer_identity_read = if selected_designer_identity_live_enabled",
        ),
        (
            "d94",
            "pov_visibility_read = pov_province_visibility_live_enabled",
        ),
        (
            "d95",
            "pov_spotted_units_read = pov_spotted_units_live_enabled",
        ),
        (
            "d97",
            "let peace_conference_read = if peace_conference_live_enabled",
        ),
    ] {
        let at = find(needle);
        assert!(
            tracker < at,
            "{domain} must be read AFTER the lane tracker exists, so its \
             bracket can be recorded (tracker at line {tracker}, read at \
             line {at})"
        );
        assert!(
            at < fanout,
            "{domain} must be read BEFORE the worker fan-out opens \
             (read at line {at}, fan-out at line {fanout})"
        );
    }

    let terminal_drain = find("publish_naval_member_epochs_after_terminal_drain(");
    assert!(
        fanout < terminal_drain,
        "d86/d90 must still follow the d49 terminal drain, which follows the \
         join (fan-out at line {fanout}, drain at line {terminal_drain})"
    );
}

#[test]
fn the_moved_reads_bracket_the_hour_before_any_worker_bracket_opens() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let mut names = vec![RAW_D47_DOMAIN];
    names.extend(AO_MOVED_DOMAINS.iter().map(|(_, _, raw)| *raw));
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&names),
        hour,
        7,
    );

    let opening_ordinals = |tag: u16| -> Vec<u64> {
        let mut found: Vec<u64> = outcome
            .timing
            .domain_clock_brackets
            .iter()
            .filter(|bracket| bracket.tag == tag)
            .filter_map(|bracket| bracket.opening.map(|observation| observation.ordinal))
            .collect();
        found.sort_unstable();
        found
    };

    let d47 = opening_ordinals(COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE);
    assert_eq!(
        d47.len(),
        1,
        "premise: d47 contributes its worker's one bracket, whose opening is \
         the upper bound the moved reads sit under; got {d47:?}"
    );
    let d47_worker = d47[0];

    for (domain, id, _) in AO_MOVED_DOMAINS {
        let ordinals = opening_ordinals(id);
        assert_eq!(
            ordinals.len(),
            1,
            "{domain} must hold exactly one clock bracket in a frame that \
             schedules it once; got {ordinals:?}"
        );
        let at = ordinals[0];
        assert!(
            at < d47_worker,
            "{domain} opened its bracket after d47's WORKER did \
             ({domain} ordinal {at}, d47 worker ordinal {d47_worker})"
        );
    }
}

#[test]
fn the_two_drained_domains_are_read_exactly_once_per_frame_under_the_new_order() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let control = compact_control_scheduling(&[
        RAW_POV_PROVINCE_VISIBILITY_LIVE_DOMAIN,
        RAW_POV_SPOTTED_UNITS_LIVE_DOMAIN,
    ]);

    for frame in 0..2u64 {
        let outcome =
            drive_compact_frame(Some(&fixture.collector_state()), &control, hour, 7 + frame);
        for (domain, id) in [
            ("d94", COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE),
            ("d95", COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE),
        ] {
            let brackets = outcome
                .timing
                .domain_clock_brackets
                .iter()
                .filter(|bracket| bracket.tag == id)
                .count();
            assert_eq!(
                brackets, 1,
                "{domain}'s slot must be drained exactly once per frame, and a \
                 drain is one bracket; frame {frame} held {brackets}"
            );
            let terminal = outcome
                .records
                .iter()
                .filter(|record| record.domain_id == id)
                .count();
            assert_eq!(
                terminal,
                1,
                "{domain} must resolve to exactly one terminal record per \
                 frame; frame {frame} emitted {terminal}; wire: {:?}",
                outcome
                    .records
                    .iter()
                    .map(|record| (record.domain_id, record.record_type))
                    .collect::<Vec<_>>()
            );
            let spans: Vec<&LaneSpan> = outcome
                .timing
                .lane_spans
                .iter()
                .filter(|span| span.tag == id)
                .collect();
            assert_eq!(
                spans.len(),
                1,
                "{domain} must record exactly one lane span per frame; frame \
                 {frame} recorded {}",
                spans.len()
            );
            assert_eq!(
                spans[0].lane, LW_MAIN_LANE,
                "{domain}'s drain still runs on the collector thread, so its \
                 span must stay tagged to the MAIN lane and out of the \
                 critical-path accounting"
            );
        }
    }
}

#[test]
fn a_refused_drain_still_names_its_reason_at_the_new_read_position() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[
            RAW_POV_PROVINCE_VISIBILITY_LIVE_DOMAIN,
            RAW_POV_SPOTTED_UNITS_LIVE_DOMAIN,
        ]),
        hour,
        7,
    );

    for (domain, id) in [
        ("d94", COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE),
        ("d95", COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE),
    ] {
        assert!(
            outcome
                .record(id, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA)
                .is_none(),
            "premise: {domain}'s drain refused in this process, so there is no \
             DATA record for the refusal to be about"
        );
        let recorded = outcome
            .trackers
            .domain_outcomes
            .get(id)
            .unwrap_or_else(|| panic!("{domain} must reach the outcome ledger"));
        assert_eq!(
            recorded.completion,
            compact_contract::domain_outcome::ReadCompletion::DidNotComplete,
            "{domain} must name why it has no rows"
        );
        assert_eq!(
            recorded.extent,
            compact_contract::domain_outcome::AnswerExtent::Unprovable,
            "{domain} publishes no DATA record at all, so nothing is known to \
             be missing from a reached extent -- what is unknown is whether \
             there was one"
        );
        assert!(
            !recorded.reason.is_empty(),
            "{domain}'s refusal must carry the producer's own text, not an \
             empty string the manifest cannot act on"
        );
    }
}

use design_catalog_context_live_lane::peace_conference_live as aq_peace;

fn aq_peace_fixture(hour: i32, conference_count: i32) -> CollectorStateFixture {
    use design_catalog_context_live_lane::peace_conference_presence as presence;
    const GAME_STATE: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x4000);
    fixture.stage_scheduler_clock_at(GAME_STATE);
    fixture
        .heap
        .write_i32(GAME_STATE + GS_CURRENT_DATE_OFFSET, hour);
    let container = GAME_STATE + presence::PEACE_CONFERENCE_CONTAINER_OFFSET;
    let elements = fixture.heap.at(GAME_STATE + 0x800) as u64;
    fixture.heap.write_u64(
        container + presence::CONTAINER_DATA_OFFSET,
        if conference_count == 0 { 0 } else { elements },
    );
    fixture.heap.write_i32(
        container + presence::CONTAINER_COUNT_OFFSET,
        conference_count,
    );
    fixture
}

#[test]
fn d97_publishes_a_measured_zero_when_the_engine_holds_no_conference() {
    let hour = fixture_game_hour_raw();
    let fixture = aq_peace_fixture(hour, 0);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_PEACE_CONFERENCE_LIVE_DOMAIN]),
        hour,
        7,
    );

    let data = outcome
        .domain_data(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE)
        .expect("d97 must emit a DOMAIN_DATA record");
    let decoded = aq_peace::decode_payload(&data.payload).expect("d97 payload decodes");
    assert_eq!(
        decoded.observation,
        Some(aq_peace::PeaceConferenceObservation::NoConferenceInProgress),
        "d97 must publish a measured zero"
    );
    assert_eq!(decoded.status_flags, aq_peace::MEASURED_EMPTY_STATUS);

    let perf = outcome
        .perf(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE)
        .expect("d97 must emit a DOMAIN_PERF record");
    assert_eq!(perf.status_flags, aq_peace::MEASURED_EMPTY_STATUS);
    assert_eq!(perf.row_count, 0, "a measured zero carries no rows");
    assert!(
        !compact_contract::compact_status_is_failure(perf.status_flags),
        "the hour is delivered, not refused"
    );
}

#[test]
fn d97_still_refuses_when_the_engine_holds_a_conference_it_never_observed() {
    let hour = fixture_game_hour_raw();
    let fixture = aq_peace_fixture(hour, 2);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_PEACE_CONFERENCE_LIVE_DOMAIN]),
        hour,
        7,
    );

    let perf = outcome
        .perf(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE)
        .expect("d97 must emit a DOMAIN_PERF record");
    assert_ne!(
        perf.status_flags & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "an unobserved live conference is a real miss"
    );
    assert_eq!(
        perf.status_flags & aq_peace::MEASURED_EMPTY_STATUS,
        0,
        "a miss must never wear the measurement bit"
    );
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE, "d97_conference_count"),
        Some(2),
        "the deciding fact must reach the trailer as a number"
    );
}

#[test]
fn d97_publishes_its_diagnostic_trailer() {
    let hour = fixture_game_hour_raw();
    let fixture = aq_peace_fixture(hour, 0);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_PEACE_CONFERENCE_LIVE_DOMAIN]),
        hour,
        7,
    );

    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE, "d97_hook_installed"),
        Some(0),
        "the test process never patches hoi4.exe"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE,
            "d97_refresh_observed_total"
        ),
        Some(0)
    );
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE);
}

#[test]
fn d85_publishes_its_diagnostic_trailer() {
    let hour = fixture_game_hour_raw();
    let fixture = d47_clock_fixture(hour);
    let outcome = drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_SELECTED_DESIGNER_IDENTITY_LIVE_DOMAIN]),
        hour,
        7,
    );

    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
            "d85_hook_installed"
        ),
        Some(0),
        "the test process never patches hoi4.exe, so d85 has no observer"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
            "d85_callback_count"
        ),
        Some(0)
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
            "d85_error_callback_ordinal"
        ),
        Some(0),
        "no site holds a failure, so the ordinal is published as its reset sentinel"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
            "d85_latched_error_scope_mask"
        ),
        Some(0)
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE,
            "d85_division_view_registrations"
        ),
        Some(0),
        "the test process never patches hoi4.exe, so no order callback ever ran"
    );
    outcome
        .assert_diagnostics_agree_with_production(COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE);
}

/// have made it `per_frame` and SUMMED an observer-lifetime counter across
#[test]
fn the_new_a40_units_are_classified_and_none_is_summed_by_accident() {
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
    for (name, expected) in [
        ("d85_hook_installed", Accumulation::Cumulative),
        ("d85_callback_count", Accumulation::Cumulative),
        (
            "d85_variant_upgrade_selection_count",
            Accumulation::Cumulative,
        ),
        ("d85_observed_families", Accumulation::Gauge),
        // Lane AX2. An observer-lifetime registration total, so it must be
        ("d85_division_view_registrations", Accumulation::Cumulative),
        ("d85_error_callback_ordinal", Accumulation::LastValue),
        ("d85_latched_error_scope_mask", Accumulation::BitwiseOr),
        ("d85_latched_error_reason_code", Accumulation::LastValue),
        ("d97_hook_installed", Accumulation::Cumulative),
        ("d97_refresh_observed_total", Accumulation::Cumulative),
        ("d97_conference_count", Accumulation::Gauge),
        (
            "peace_conference_no_conference_frames",
            Accumulation::PerFrame,
        ),
    ] {
        assert_eq!(unit_accumulation(name), expected, "{name}");
    }
}

/// D95's census and site-entry counts are process-lifetime totals published as
#[test]
fn every_d95_census_unit_is_classified_cumulative_and_a_new_sibling_is_undeclared() {
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
    for name in [
        "d95_hooks_installed",
        "d95_site_entries_intel_getter",
        "d95_site_entries_land_frame",
        "d95_site_entries_land_eligibility",
        "d95_site_entries_taskforce_frame",
        "d95_land_candidates_seen",
        "d95_land_rejected_unreadable_handle",
        "d95_land_rejected_below_intel_threshold",
        "d95_land_rejected_by_eligibility",
        "d95_land_rejected_taskforce_state",
        "d95_land_abandoned_pending",
        "d95_land_admitted",
        "d95_land_frames_without_enumeration",
        "d95_land_frames_producer_gate_closed",
        "d95_taskforce_candidates_seen",
        "d95_taskforce_rejected_unreadable_handle",
        "d95_taskforce_rejected_below_intel_threshold",
        "d95_taskforce_rejected_by_eligibility",
        "d95_taskforce_rejected_taskforce_state",
        "d95_taskforce_abandoned_pending",
        "d95_taskforce_admitted",
        "d95_taskforce_frames_without_enumeration",
        "d95_taskforce_frames_producer_gate_closed",
        "d95_taskforce_frames_source_collection_empty",
        "d95_land_admitted_rows_lost_to_replacement",
        "d95_taskforce_admitted_rows_lost_to_replacement",
        "d95_next_sequence",
    ] {
        assert_eq!(unit_accumulation(name), Accumulation::Cumulative, "{name}");
    }
    assert_eq!(
        unit_accumulation("d95_a_unit_nobody_registered"),
        Accumulation::Undeclared,
        "an unregistered d95 sibling must be undeclared, never silently summed"
    );
}

#[test]
fn every_unit_d95_declares_is_registered_and_reaches_the_emit_site() {
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation, UnitRole};
    let declared =
        super::super::d95_declared_diagnostic_units(&spotted_units_live_lane::diagnostics());
    assert_eq!(
        declared.len(),
        27,
        "one installed flag, four site-entry counts, ten census arms per surface, \
         the task-force-only measured-empty count, and the frame-id allocator"
    );
    for unit in &declared {
        assert_eq!(
            unit_accumulation(unit.name),
            Accumulation::Cumulative,
            "{} is a process-lifetime total and must not be summed per frame",
            unit.name
        );
        assert!(
            unit.name.starts_with("d95_"),
            "{} must sit inside the stateful d95 family",
            unit.name
        );
    }
    let deciding = declared
        .iter()
        .filter(|unit| unit.role == UnitRole::Deciding)
        .map(|unit| unit.name)
        .collect::<Vec<_>>();
    assert_eq!(
        deciding,
        vec![
            "d95_hooks_installed",
            "d95_land_candidates_seen",
            "d95_land_admitted_rows_lost_to_replacement",
            "d95_taskforce_candidates_seen",
            "d95_taskforce_admitted_rows_lost_to_replacement",
        ],
        "the deciding units are the ones a reader consults first: was the \
         observer patched in, did either producer enumerate anything, and did \
         either surface admit rows that never reached a drain"
    );

    let production = include_str!("../../attach_log.rs");
    let emit = production
        .split_once("domain_id: COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE,")
        .expect("the d95 perf emit site")
        .0;
    let call = emit
        .rsplit_once("append_compact_domain_perf_record")
        .expect("the d95 perf record appender")
        .1;
    assert!(
        call.starts_with("_with_declared_diagnostics"),
        "d95's perf record must be appended WITH its declared diagnostics; \
         the plain appender publishes no trailer at all, which is the state \
         every capture through t25 was in"
    );
    let arguments = production
        .split_once("domain_id: COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE,")
        .expect("the d95 perf emit site")
        .1
        .chars()
        .take(600)
        .collect::<String>();
    assert!(
        arguments.contains("&d95_declared_diagnostic_units(&spotted_units_live_lane::diagnostics()),"),
        "and the units it appends must be d95's own, read through the lane's own          published diagnostics"
    );
}

fn perf_diag_value(perf: &DomainPerfView, name: &str) -> u64 {
    perf.diagnostics
        .iter()
        .find(|(unit, _)| unit == name)
        .map(|(_, value)| *value)
        .unwrap_or_else(|| panic!("perf record lacks {name}"))
}
