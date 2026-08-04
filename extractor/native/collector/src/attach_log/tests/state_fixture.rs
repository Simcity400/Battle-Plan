//! Offline frame fixture: drive the PRODUCTION frame builder with a
//! populated `CollectorState`, with no game process.
//!
//! # Why this exists
//!
//! `build_compact_extraction_gate_records` is the one function every
//! emitted domain passes through, and until now every test that reached it
//! handed it either `None` or [`empty_collector_state`] — a state whose
//! `base` and `date_ptr` are both zero, so every guarded read fails closed
//! and every domain reports `SOURCE_UNAVAILABLE`. Questions of the form
//! "what does domain X actually put on the wire when its roots ARE
//! readable" therefore had no offline answer at all and kept being
//! deferred to a live patched `hoi4.exe`.
//!
//! # The trick, and where it comes from
//!
//! `domain57_game_speed_reader_round_trips_and_fails_closed` already
//! proves the core move in-tree: allocate a real block, then choose
//! `module_base = block.addr() - RVA` so the reader's `module_base + RVA`
//! lands exactly on the block. [`ModuleImageWindow`] generalises that from
//! ONE rva to a contiguous WINDOW of them, so a whole domain's
//! module-relative roots can be staged at once and read through the same
//! guarded readers production uses.
//!
//! # What this fixture is
//!
//! It stages MEMORY and then calls the real readers. It reimplements no
//! reader and no encoder: every byte asserted on is a byte the production
//! frame builder appended, framed back with `compact_contract`'s own
//! parser and held to `compact_contract::record_semantics`. If a
//! production READ is broken, the tests below go red — see the mutation
//! note on each one.
//!
//! # What this fixture does NOT prove -- read this before planning on it
//!
//! * **Offset and RVA constants.** Staging necessarily writes through the
//!   same `build_profile` constant the reader then reads through, so a
//!   WRONG constant moves both sides together and stays invisible. This
//!   was confirmed by mutation: shifting `CSHIP_OFF_HANDLE` left every
//!   test green. Constants are answered by RE evidence, not by this
//!   fixture; what the fixture answers is reader and frame LOGIC relative
//!   to the compiled constants.
//! * **Wire encoders.** Where a test compares an emitted payload to
//!   `encode_x(rows)`, the encoder is the same one production called, so
//!   an encoder defect moves both sides together. Those comparisons pin
//!   the frame builder's choice of ROWS and SCALARS (hour, wall, status,
//!   counts), not the byte layout — the layout is the wire-format lanes'
//!   subject. The one decoder here, [`DomainPerfView::parse`], is called
//!   out at its own definition.
//! * **Anything needing an installed a729 inline hook, a real GUI
//!   callback, or a running game clock.** Those genuinely still require a
//!   live patched `hoi4.exe`.
//! * **RVAs outside the staged window.** They are not mapped by this
//!   fixture; they are not guaranteed to be UNMAPPED either (the process
//!   owns other memory), so a domain must never be assumed to work
//!   because some unrelated allocation happened to answer a read.

use super::*;

// ---------------------------------------------------------------------
// Staging primitives
// ---------------------------------------------------------------------

/// A dynamically sized, alignment-controlled block of ordinary heap memory
/// that the production guarded readers can traverse.
///
/// `AlignedBytes<N>` covers the fixed-size case and is used everywhere in
/// this file's sibling tests; this is its runtime-sized sibling, needed
/// because a module image window's length is derived from build-profile
/// RVAs rather than known at compile time.
pub(super) struct FixtureRegion {
    bytes: Vec<u8>,
    base_offset: usize,
    len: usize,
}

impl FixtureRegion {
    /// A zeroed region of `len` bytes whose base address is a multiple of
    /// `align`. Over-allocates by `align` and picks the aligned interior
    /// start, so no unsafe allocation dance is needed.
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

    /// 8-aligned like every engine object the readers walk.
    pub(super) fn new(len: usize) -> FixtureRegion {
        FixtureRegion::with_alignment(len, 8)
    }

    pub(super) fn addr(&self) -> usize {
        self.bytes.as_ptr() as usize + self.base_offset
    }

    /// Address of `offset` inside the region, bounds-checked so a fixture
    /// typo cannot silently hand a reader an address outside the staging
    /// area (which would read whatever the test process happens to own
    /// there and could make a broken reader look healthy).
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

    pub(super) fn write_i32(&mut self, offset: usize, value: i32) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    pub(super) fn write_u64(&mut self, offset: usize, value: u64) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

}

/// A synthetic module image WINDOW.
///
/// One readable block placed so that `module_base + rva` lands inside it
/// for every RVA the fixture declared, generalising the single-RVA trick
/// `domain57_game_speed_reader_round_trips_and_fails_closed` already
/// proves. `module_base` is page-aligned, like a real PE image base.
///
/// The window's length is `max(rva) + slack - page_floor(min(rva))`, so a
/// caller that only needs d57's game-state singleton pays a few pages
/// while a caller that also needs the profile sentinels near the image
/// head pays the full span between them. That cost is the honest one: it
/// is exactly the address range the engine's own image spans between the
/// staged roots.
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

/// A `CollectorState` whose module base points at a staged
/// [`ModuleImageWindow`] and whose engine objects live in a staged
/// [`FixtureRegion`].
///
/// Both regions are owned by the fixture. `CollectorState` carries RAW
/// integer addresses, not borrows, so nothing checks this for you: the
/// fixture MUST outlive every `CollectorState` minted from it, or the
/// readers will walk freed memory. Keep the fixture (and any
/// [`ModuleImageWindow`] handed out separately) bound in the test.
pub(super) struct CollectorStateFixture {
    pub(super) module: ModuleImageWindow,
    pub(super) heap: FixtureRegion,
    date_ptr: u64,
    country_database_ptr: u64,
    // `CountryIdentityCache` is deliberately not `Clone` in production, so
    // the fixture keeps the STAGED INPUTS and mints a fresh cache on every
    // `collector_state()` call instead of cloning one.
    country_roster: Option<(&'static str, usize, Vec<CountryObjectRow>)>,
    profile_sentinels_staged: bool,
}

impl CollectorStateFixture {
    /// Stage a module window over `rvas` and a `heap_len`-byte engine
    /// object arena.
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

    /// Point `state.date_ptr` at a heap offset (the CGameState root every
    /// non-module-relative reader walks from).
    pub(super) fn set_date_ptr_at(&mut self, heap_offset: usize) -> &mut Self {
        self.date_ptr = self.heap.at(heap_offset) as u64;
        self
    }

    /// Stage the frame's AUTHENTICATED SCHEDULER CLOCK on a heap CGameState:
    /// `date_ptr` and the module image's CGameState singleton slot are made
    /// to name the same object, which is the pairing
    /// `read_authenticated_scheduler_clock` insists on.
    ///
    /// Without it the frame builder replaces every domain's records with a
    /// `ClockUnreadable` DOMAIN_GAP, so this is a precondition for asking
    /// ANY frame-level question, not an optional extra.
    pub(super) fn stage_scheduler_clock_at(&mut self, heap_offset: usize) -> &mut Self {
        let game_state = self.heap.at(heap_offset) as u64;
        self.date_ptr = game_state;
        self.module.write_u64_at_rva(
            compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
            game_state,
        );
        // The clock must READ the fixture's label hour, not the heap's zero
        // fill. Every reader bracket opens and closes against this word, and
        // since `HOUR_LABEL_MISMATCH_POLICY` became `Refuse`, a bracket that
        // reads hour 0 under a frame labelled `fixture_game_hour_raw()` is a
        // mislabelled read and the domain is (correctly) replaced with a gap
        // -- which silently deleted the very records these fixtures assert
        // on. Tests that stage a DIFFERENT hour on purpose overwrite this
        // word after calling this method.
        self.heap.write_i32(
            heap_offset + GS_CURRENT_DATE_OFFSET,
            fixture_game_hour_raw(),
        );
        self
    }

    /// Supply a complete, verified country roster.
    ///
    /// This is what decides `COMPACT_STATUS_NO_SCALAR_SOURCE` at the top of
    /// the frame, which in turn is a failure bit that short-circuits most
    /// country-scoped readers. `status` must be one of the four strings
    /// `CountryIdentityCache::has_scalar_source` accepts.
    pub(super) fn set_verified_country_roster(
        &mut self,
        status: &'static str,
        country_vtable: usize,
        rows: Vec<CountryObjectRow>,
    ) -> &mut Self {
        self.country_roster = Some((status, country_vtable, rows));
        self
    }

    /// Write the real build-profile signature bytes at
    /// `base + C_GAME_STATE_DAILY_UPDATE_OFFSET` and
    /// `base + GET_LONG_STRING_OFFSET`.
    ///
    /// Several readers do not trust `state.daily_match` on its own: they
    /// re-run `sentinel_matches` against those two addresses before they
    /// will touch world data (`compact_country_preferred_tactic_*`,
    /// `A17CurrentProcessSource`). Staging the bytes is what makes those
    /// gates reachable offline. Both constants and both byte patterns come
    /// from `compact_contract::build_profile`; nothing here is invented.
    ///
    /// The caller must have declared both RVAs to [`Self::new`], which is
    /// what makes this window span the ~51 MB between the image head and
    /// the CGameState singleton.
    pub(super) fn stage_profile_sentinels(&mut self) -> &mut Self {
        self.module
            .write_bytes_at_rva(C_GAME_STATE_DAILY_UPDATE_OFFSET, &DAILY_UPDATE_SENTINEL);
        self.module
            .write_bytes_at_rva(GET_LONG_STRING_OFFSET, &GET_LONG_STRING_SENTINEL);
        self.profile_sentinels_staged = true;
        self
    }

    /// The state the production frame builder will be handed.
    ///
    /// `daily_addr` / `long_string_addr` are derived from `base` exactly
    /// as `CollectorState::resolve` derives them, so the several
    /// `state.daily_addr != state.base + OFFSET` consistency gates see a
    /// self-consistent state rather than the `0` that
    /// `compact_naval_fixture` leaves them at.
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

// ---------------------------------------------------------------------
// Driving the production frame
// ---------------------------------------------------------------------

/// One record the frame builder appended, framed back out with
/// `compact_contract`'s own parser.
#[derive(Clone, Debug)]
pub(super) struct FrameRecord {
    pub(super) sequence: u64,
    pub(super) game_hour_raw: i32,
    pub(super) domain_id: u16,
    pub(super) record_type: u16,
    pub(super) record_flags: u32,
    pub(super) payload: Vec<u8>,
}

/// The DOMAIN_PERF payload the frame builder emitted for one domain.
///
/// This is the one hand-written decoder in the fixture, and it is here
/// because production has no total one: `compact_perf_diagnostic_unit_names`
/// is deliberately LOSSY (it drops zero-valued units and caps at six per
/// domain) because it feeds a size-bounded failure log, and the zero-valued
/// units are exactly what a "this did not happen" assertion needs.
///
/// Two things hold it to the shipped grammar rather than to this file's
/// idea of it: `validate_compact_record_semantics` has already accepted
/// every record in a [`FrameOutcome`], and
/// [`FrameOutcome::assert_diagnostics_agree_with_production`] cross-checks
/// the non-zero units against the production reader.
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
                let name = String::from_utf8_lossy(&payload[at + 12..at + 12 + name_len]).to_string();
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

/// Everything one production frame produced.
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

    /// Hold this file's PDU decoder to the production one.
    ///
    /// REWRITTEN when the production reader became
    /// `compact_perf_diagnostic_units`. The old premise -- "it emits
    /// `name=value` for NON-ZERO units only, up to six" -- described two
    /// behaviours the producer migration removed as defects: dropping zeros
    /// made a domain whose failure stage genuinely is `0` indistinguishable
    /// from one that emitted nothing, and keeping the FIRST six chose the
    /// survivors by statement order in the producer. Selection now happens
    /// once, later, over every offender at the same time, and this reader's
    /// only job is to read the trailer faithfully.
    ///
    /// So the two decoders are now compared over their WHOLE output rather
    /// than over a filtered prefix, which is a stronger agreement than the
    /// one this assertion used to make. It stays falsifiable: change either
    /// decoder's field widths, unit ordering, name slicing, or its handling
    /// of a zero-valued unit, and the two lists stop matching.
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

    /// A named PDU1 diagnostic unit, searched across EVERY perf record the
    /// frame emitted for `domain_id`.
    ///
    /// The reserved probe domain 250 carries several independent perf
    /// records in one frame (read-counter probe, lane-wall probe, sink
    /// probe, and an attribution probe that is emitted only when the
    /// process-global bound-trip / status-origin registries are non-empty).
    /// Which of them comes first therefore depends on what OTHER tests in
    /// the binary have done, so a lookup that takes the first probe record
    /// is load-flaky. Scan them all.
    pub(super) fn diagnostic_unit(&self, domain_id: u16, name: &str) -> Option<u64> {
        self.records
            .iter()
            .filter(|record| {
                record.domain_id == domain_id
                    && record.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
            })
            .find_map(|record| {
                DomainPerfView::parse(&record.payload)
                    .diagnostics
                    .into_iter()
                    .find(|(unit, _)| unit == name)
                    .map(|(_, value)| value)
            })
    }

    pub(super) fn frame_end(&self) -> &FrameRecord {
        self.records
            .last()
            .filter(|record| record.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END)
            .expect("the final record of a frame is its FRAME_END")
    }

    /// Every domain that emitted a DOMAIN_DATA record this frame.
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

    /// The spawn -> return wall the frame's own lane tracker recorded for
    /// `tag`, in microseconds from the start of the worker-read phase.
    ///
    /// This is the same `LaneSpan` production carries in its `lw_*` probe
    /// record, so a question of the form "does this worker ever return"
    /// has an offline answer for every domain the fixture can drive.
    pub(super) fn lane_span(&self, tag: u16) -> Option<&LaneSpan> {
        self.timing.lane_spans.iter().find(|span| span.tag == tag)
    }
}

/// Run ONE production frame against `state` and frame the bytes back out.
///
/// The `page_cache_new_frame()` call is not fixture scaffolding: the live
/// capture loop makes exactly this call at the top of every sample, before
/// it calls `build_compact_extraction_gate_records`, and the good-page
/// cache would otherwise serve a previous test's confirmations.
pub(super) fn drive_compact_frame(
    state: Option<&CollectorState>,
    control: &CollectorControl,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> FrameOutcome {
    let mut trackers = CompactPerfGateTrackers::default();
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
        &mut trackers,
        None,
        None,
        None,
        None,
        &mut OutsideBuildSpans::default(),
    );

    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let header = compact_contract::parse_compact_record_header(&bytes[offset..])
            .expect("the frame builder emits well-framed compact records");
        let payload_start = offset + compact_contract::COMPACT_RECORD_HEADER_LEN;
        let payload = bytes[payload_start..payload_start + header.payload_len].to_vec();
        // Every record is held to the SHIPPED grammar, not to this
        // fixture's idea of it. A frame that emits a malformed perf
        // trailer or a mis-typed frame end fails here rather than in a
        // later, vaguer assertion.
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
        records.push(FrameRecord {
            sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            record_flags: header.record_flags,
            payload,
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

/// An empty image window big enough to carry the CGameState singleton
/// slot. Build this FIRST: an arena fixture that stages vtable pointers as
/// `module_base + RVA` has to be told the base before it stages them.
pub(super) fn scheduler_clock_image() -> ModuleImageWindow {
    ModuleImageWindow::spanning(
        &[compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA],
        0x1000,
    )
}

/// Point the image's CGameState singleton slot at `state.date_ptr` and
/// give `state` the image's base, derived exactly as
/// `CollectorState::resolve` derives it. The caller must keep the image
/// alive for as long as the state is used.
///
/// This is not decoration. `read_authenticated_scheduler_clock` opens and
/// closes a bracket around every reader by reading
/// `module_base + DOMAIN57_CGAMESTATE_SINGLETON_RVA`, requiring that
/// singleton to be a heap-aligned pointer whose `+ GS_CURRENT_DATE_OFFSET`
/// is exactly the frame's date address — i.e. the singleton slot and
/// `state.date_ptr` must name the SAME object. When that read fails, the
/// frame builder does not merely lose timing information: it REPLACES
/// every one of that domain's records with a `ClockUnreadable` DOMAIN_GAP.
///
/// That is why an arena-only fixture such as `compact_naval_fixture`, with
/// its unmapped `0x1_4000_0000` module base, cannot answer any
/// frame-level question on its own however well its engine objects are
/// staged — a fact this lane found by driving the frame and reading the
/// gaps that came out.
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

/// The naval world `compact_naval_fixture` stages, with a REAL module
/// image behind it so the frame's authenticated scheduler clock resolves
/// and the domain's records survive to the wire.
pub(super) fn naval_frame_fixture() -> (Box<AlignedBytes<0x9000>>, CollectorState, ModuleImageWindow)
{
    let mut image = scheduler_clock_image();
    let (mut arena, mut state) = compact_naval_fixture_at_module_base(image.module_base());
    bind_scheduler_clock(&mut image, &mut state);
    // The clock must READ the fixture's label hour. The arena's date word sat
    // at its zero fill, which authenticated fine and read hour 0 -- invisible
    // while `HOUR_LABEL_MISMATCH_POLICY` was `CountOnly`, but under `Refuse` a
    // bracket that reads hour 0 against a real label is a mislabelled read and
    // every domain in the frame is (correctly) replaced with a gap.
    let date_offset = (state.date_ptr as usize)
        .checked_sub(arena.addr())
        .expect("the naval date root lives inside its own arena")
        + GS_CURRENT_DATE_OFFSET;
    arena.write_u32(date_offset, fixture_game_hour_raw() as u32);
    (arena, state, image)
}

/// A fixture whose two FRAME-SCOPED failure bits are clear: profile
/// sentinels staged (else `PROFILE_MISMATCH`) and a verified country
/// roster supplied (else `NO_SCALAR_SOURCE`). The caller still has to
/// `stage_scheduler_clock_at` a heap CGameState, or the frame will replace
/// its records with clock gaps.
///
/// Both of those bits are in `COMPACT_STATUS_FAILURE_MASK`, and a great
/// many readers open with `if compact_status_is_failure(status_flags)
/// { return unavailable }` — so a fixture that skips either one measures
/// the short-circuit rather than the domain. Clearing them is what costs
/// the ~51 MB window: the sentinels live near the image head and the
/// CGameState singleton near its tail, and one module base has to span
/// both.
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

/// A control that schedules exactly `domains` and nothing else.
pub(super) fn compact_control_scheduling(domains: &[&str]) -> CollectorControl {
    let mut control = raw_test_control();
    control.stream_format = CollectorStreamFormat::Compact;
    control.scheduled_domains = domains.iter().map(|name| (*name).to_string()).collect();
    control
}

/// An hour label the calendar accepts, so a frame is never rejected for a
/// reason unrelated to what the test is asking about.
pub(super) fn fixture_game_hour_raw() -> i32 {
    compact_contract::calendar::DATE_EPOCH_CONST
        .checked_add(16_959_360i32)
        .expect("fixture hour is representable")
}

// ---------------------------------------------------------------------
// Demonstration 1 -- a domain reaching the frame builder with POPULATED
// state emits a DOMAIN_DATA record carrying rows read out of memory.
// ---------------------------------------------------------------------

/// The headline question this lane exists to make answerable offline:
/// can a `DOMAIN_DATA` record carrying real rows be produced with no game
/// process?
///
/// `compact_naval_fixture` already stages a CGameState -> country -> fleet
/// -> task force -> ship chain and is already read by the production
/// `discover_compact_naval_oob_roots` in sibling tests. What was missing
/// was driving the FRAME with it: the frame builder spawns d29's worker,
/// joins it, and appends the perf + data pair. Nothing here reimplements
/// the ship reader; the assertions are on the bytes the frame appended.
///
/// What production change makes this fail: any break in
/// `sample_compact_ships_native` / `discover_compact_naval_oob_roots` /
/// the d29 emission site. Verified by mutation, see the lane report.
#[test]
fn populated_state_makes_the_ships_domain_emit_domain_data_rows_offline() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = compact_control_scheduling(&[RAW_SHIPS_DOMAIN]);
    let outcome = drive_compact_frame(
        Some(&state),
        &control,
        fixture_game_hour_raw(),
        7,
    );

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
    // Two ships were staged (`write_fixture_ship` x2) under one task force.
    assert_eq!(perf.row_count, 2, "both staged ships were observed");
    assert_eq!(perf.unit_count, 1, "one staged task force was observed");
    assert_eq!(perf.changed_rows, 2, "both rows are new to a cold tracker");

    let data = outcome
        .domain_data(COMPACT_DOMAIN_SHIPS)
        .expect("d29 emitted a DOMAIN_DATA record");
    // Two links, and the chain is complete:
    //
    //   staged bytes -> production reader rows   (asserted field by field)
    //   reader rows  -> the payload the frame appended  (production encoder)
    //
    // The first link is what makes this test sensitive to a reader that
    // reads the WRONG FIELD; an equality against reader output alone would
    // be blind to exactly that, since both sides would move together.
    let roots = discover_compact_naval_oob_roots(&state).expect("staged naval roots");
    let native = sample_compact_ships_from_roots(state.base, &roots, true).expect("ship rows");
    // `write_fixture_ship(arena, base, slot, instance_id, category, sunk, definition)`
    // stages strength/organisation/experience as fixed offsets from the
    // instance id, under task force 617 owned by tag_db_id 1.
    let staged_row = |instance_id: u32, category_enum: u32, sunk_convoys: u32, definition: &str| {
        CompactShipRow {
            instance_id,
            deleted: false,
            task_force_id: Some(617),
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
    // What this pins is the frame builder's choice of rows and scalars:
    // `expected` is built with the SAME production encoder the frame used,
    // so an encoder-layout defect would move both sides together. Byte
    // layout is the wire-format lanes' subject, not this fixture's.
    assert_eq!(
        data.payload, expected,
        "the frame's d29 payload must be the production encoding of those rows,          with the frame's own hour/wall/status/count scalars"
    );

    // The frame is sealed and the domain is counted. (The frame-end
    // record's DOMAIN is not asserted: `validate_compact_record_semantics`
    // has already rejected any other, so it could not fail here.)
    assert_eq!(
        outcome.frame_end().record_type,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END
    );
    assert_eq!(outcome.timing.emitted_domains, 1);
    assert_eq!(outcome.domains_with_data(), vec![COMPACT_DOMAIN_SHIPS]);

    // The rest of what the fixture surfaces is bound to the same bytes,
    // so a later lane can trust these views without re-deriving them:
    //   * headers are consecutively sequenced from the requested start
    //     and all carry the requested hour label,
    //   * the perf view's wall/diagnostic halves came off the wire.
    assert_eq!(
        outcome.records.iter().map(|record| record.sequence).collect::<Vec<_>>(),
        (1..=outcome.records.len() as u64).collect::<Vec<_>>()
    );
    assert!(outcome.next_sequence > outcome.records.len() as u64);
    for record in &outcome.records {
        assert_eq!(record.game_hour_raw, fixture_game_hour_raw());
    }
    assert_eq!(perf.elapsed_wall_ms, 7, "the perf record carries the frame's wall");
    assert_eq!(perf.changed_unit_count, 2);
    assert!(perf.read_time_us <= outcome.timing.frame_time_us);
    let d29_perf_record = outcome
        .record(COMPACT_DOMAIN_SHIPS, COMPACT_RECORD_TYPE_DOMAIN_PERF)
        .expect("d29 perf record");
    assert_ne!(
        d29_perf_record.record_flags & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        0,
        "d29 always attaches its PDU1 trailer, so the flag must be set"
    );
    assert_eq!(
        perf.diagnostics,
        vec![
            ("domain29_source_unavailable".to_string(), 0u64),
            ("domain29_invalid_task_force_rows".to_string(), 0u64),
        ],
        "the fixture's diagnostic view is the units d29 actually emitted"
    );
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_SHIPS);
}

/// The first negative control for the test above: the SAME frame and the
/// SAME control, with the scheduler clock readable (so the records are not
/// replaced by gaps) but the BUILD PROFILE unverified.
///
/// `discover_compact_naval_oob_roots` opens with
/// `state.base == 0 || !state.daily_match || !state.long_string_match`, so
/// this is the profile gate refusing, not the naval walk failing — the two
/// are worth keeping apart, and this test is named for the one it actually
/// exercises. The pair of records is still emitted (the wire contract
/// requires it), typed source-unavailable, with no rows.
#[test]
fn a_state_whose_build_profile_is_unverified_makes_d29_typed_source_unavailable() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
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
        ]
    );
    // This is the cross-check that has teeth: `domain29_source_unavailable`
    // is 1 here, so both decoders have a unit to disagree about. In the
    // positive test above every unit is zero, which the production reader
    // drops by design, so its cross-check compares two empty lists.
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_SHIPS);
    assert!(outcome.domain_data(COMPACT_DOMAIN_SHIPS).is_some());
}

/// The other negative control, and the single most load-bearing fact this
/// lane learned: WITHOUT a readable module image the frame builder does
/// not merely lose timing — `replace_rejected_domain_records_with_gaps`
/// DELETES the domain's perf and data records and substitutes one
/// `ClockUnreadable` DOMAIN_GAP.
///
/// So `compact_naval_fixture` on its own, however well it stages engine
/// objects, can never answer a frame-level question: its `0x1_4000_0000`
/// module base is unmapped, `read_authenticated_scheduler_clock` returns
/// `None`, and everything it staged is discarded before the wire. This
/// test pins that behaviour so a later lane recognises the shape instead
/// of concluding the domain is broken.
#[test]
fn without_a_readable_module_image_every_domain_record_becomes_a_typed_clock_gap() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};

    // The UNBOUND arena fixture: staged world, unmapped module base.
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
        .record(COMPACT_DOMAIN_SHIPS, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP)
        .expect("d29's records were replaced by one typed gap");
    let decoded = DomainGapRecordV1::decode(&gap.payload).expect("shipped gap payload");
    assert_eq!(decoded.reason, DomainGapReason::ClockUnreadable);
    assert_eq!(decoded.target_hour_raw, fixture_game_hour_raw());
}

// ---------------------------------------------------------------------
// Demonstration 2 -- module-relative roots, the capability that needed
// the synthetic module image.
// ---------------------------------------------------------------------

/// d57 reads its roots at `state.base + RVA`, which is why no prior test
/// could drive it: `compact_naval_fixture` sets `base` to the unmapped
/// literal `0x1_4000_0000`, and `empty_collector_state` sets it to `0`
/// (which d57 rejects outright at its `module_base == 0` guard).
///
/// With a staged [`ModuleImageWindow`] the game-state singleton slot at
/// `DOMAIN57_CGAMESTATE_SINGLETON_RVA` is real memory, so
/// `read_domain57_game_speed` resolves and d57 pushes a GameSpeed anchor
/// row into the frame it encodes. The row's value is the byte the fixture
/// staged.
///
/// What production change makes this fail: breaking
/// `read_domain57_game_speed`'s singleton deref, its `0..=4` clamp, or the
/// d57 game-speed row push. Verified by mutation, see the lane report.
#[test]
fn synthetic_module_image_lets_domain57_read_game_speed_from_base_relative_memory() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    const GAME_STATE_AT: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x2000);
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);

    for staged_speed in 0..=p::DOMAIN57_MAX_GAME_SPEED {
        fixture
            .heap
            .write_i32(GAME_STATE_AT + p::DOMAIN57_CGAMESTATE_GAME_SPEED_INDEX, staged_speed);
        let state = fixture.collector_state();

        let control = compact_control_scheduling(&[RAW_SESSION_MULTIPLAYER_METADATA_LIVE_DOMAIN]);
        let outcome = drive_compact_frame(
            Some(&state),
            &control,
            fixture_game_hour_raw(),
            7,
        );

        let data = outcome
            .domain_data(COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE)
            .expect("d57 emitted a DOMAIN_DATA record");
        let frame = outcome
            .trackers
            .d57_terminal_frame
            .as_ref()
            .expect("d57 published its terminal frame");
        // The tracker frame IS what went on the wire: prove it with the
        // production encoder rather than trusting the pairing.
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

/// The negative control: the same window, the same frame, the same clock,
/// but a game-speed word OUTSIDE the engine's `0..=4` range — the shape a
/// torn read produces. d57 must fail closed with no row at all rather than
/// clamp or coerce the value.
///
/// Without this, the test above would still pass if the reader invented a
/// speed of 0, and `staged_speed == 0` is one of the cases it covers.
#[test]
fn an_out_of_range_speed_word_makes_domain57_fail_closed_not_read_zero() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
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
    let outcome = drive_compact_frame(
        Some(&state),
        &control,
        fixture_game_hour_raw(),
        7,
    );

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
    // The frame's SOURCE_UNAVAILABLE bit is deliberately NOT asserted here.
    // d57's other roots (players, pause clock) are unstaged in this fixture
    // and set it too, so the bit would be present whether or not the speed
    // clamp did its job — it would pass for a reason this test is not
    // about. The absent row above is the discriminating observation.
}

// ---------------------------------------------------------------------
// Demonstration 3 -- the profile sentinels, and the worker walls.
// ---------------------------------------------------------------------

/// Several readers refuse to touch world data until they have re-read the
/// build signature bytes themselves — `state.daily_match` alone is not
/// enough for them. `stage_profile_sentinels` puts the real
/// `build_profile` byte patterns at the real RVAs, and this test proves
/// the shipped gate function accepts the result.
///
/// It is asserted through the shape those gates use — `sentinel_matches`
/// on both addresses derived from `base` — so a change to either address
/// derivation or to the sentinel comparison flips it. It does NOT clear
/// those gates on its own: they also require a non-zero `date_ptr` and
/// `country_database_ptr`, which a caller stages separately.
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

    // Before staging: the state is self-consistent but unsigned.
    let unsigned = fixture.collector_state();
    assert!(!sentinel_matches(unsigned.daily_addr, &DAILY_UPDATE_SENTINEL));
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

/// `state.date_ptr` is the OTHER root every non-module-relative reader
/// walks from, and the frame builder reads it itself before any worker
/// spawns: the acceptance skew is `|read_i32_stable(date_ptr +
/// GS_CURRENT_DATE_OFFSET) - game_hour_raw|`, published as the
/// `lw_hour_label_skew_abs` diagnostic unit on the lane-wall probe record.
///
/// So a staged date root is directly observable on the wire, which makes
/// `set_date_ptr_at` a checkable capability rather than a claim.
///
/// What production change makes this fail: dropping the `+
/// GS_CURRENT_DATE_OFFSET` term, replacing `abs_diff` with a signed
/// difference, coercing an unreadable date to a zero skew, or dropping
/// either lane-wall unit.
#[test]
fn a_staged_date_root_is_read_by_the_frames_own_acceptance_skew_probe() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    use compact_contract::build_profile as p;

    const GAME_STATE_AT: usize = 0x100;
    let hour = fixture_game_hour_raw();

    // Unstaged: date_ptr is 0, the read fails closed, and the frame says
    // so with the MISSING unit rather than reporting a zero skew.
    let missing = CollectorStateFixture::new(&[p::DOMAIN57_CGAMESTATE_SINGLETON_RVA], 0x1000, 0x1000);
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

    // Staged: the frame reads the word the fixture wrote and reports the
    // exact distance from the hour label it was handed.
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

/// `COMPACT_STATUS_NO_SCALAR_SOURCE` is a FRAME-scoped failure bit: it is
/// decided once, before any worker spawns, from
/// `state.country_identity.has_scalar_source()`, and it fans out to every
/// emitted domain. Which roster the fixture supplies therefore changes
/// what every scheduled domain can do — so the fixture has to be able to
/// set it, and this test pins that it actually does.
///
/// What production change makes this fail: relaxing
/// `has_scalar_source` (it requires an accepted status string, a non-zero
/// vtable AND a non-empty roster), or dropping the NO_SCALAR_SOURCE
/// stamp at the top of the frame.
#[test]
fn the_fixtures_country_roster_decides_the_frames_no_scalar_source_bit() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
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

/// The second wall this lane was asked to remove: "this cannot be proven
/// without a live patched `hoi4.exe` — or one capture recording each
/// worker's spawn -> return wall and outcome."
///
/// For every domain the fixture can drive, the frame's own `FrameLaneTracker`
/// records exactly that pair offline: a span per spawned worker, and the
/// worker's outcome in the perf record it produced. This test binds the
/// two together — including the outcome being a SUCCESSFUL read, since
/// production emits the same perf/data pair for a source-unavailable one —
/// and binds the spans to the `lw_worker_span_us` unit that actually
/// reaches the wire, so a future off-frame-worker question can be asked
/// here first.
#[test]
fn a_driven_frame_records_each_workers_spawn_to_return_wall_and_outcome() {
    // This test reaches the process-global bound-trip registry (a reservation
    // through `drive_compact_frame`, or a recorded trip). The registry has ONE
    // reservation slot and a drain resets EVERY counter, so an unguarded test
    // racing a guarded one is a collision or a stolen count, not a stale read.
    // Taken FIRST so it drops LAST, after any tracker or `FrameOutcome` that
    // still owns the reservation. See
    // `compact_contract::bound_trips::lock_registry_for_tests`.
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let (_arena, state, _image) = naval_frame_fixture();
    let control = compact_control_scheduling(&[RAW_SHIPS_DOMAIN, RAW_OOB_COMMAND_DOMAIN]);
    let outcome = drive_compact_frame(
        Some(&state),
        &control,
        fixture_game_hour_raw(),
        7,
    );

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
        // The outcome half of the pair. Record PRESENCE alone proves
        // nothing — production emits perf+data for a failed read too — so
        // the outcome asserted is a clean status.
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

    // The spans above are the frame's internal view. Bind them to what
    // actually reaches a capture: the lane-wall probe's worker-phase span
    // must cover every worker's return.
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

// ---------------------------------------------------------------------
// Demonstration 6 -- the d47 prewarm's clock bracket, driven through the
// PRODUCTION frame builder.
//
// `d47_root_hour_tests` proves the helper functions
// (`domain47_prewarm_clock_verdict`, `domain47_apply_prewarm_clock_verdict`,
// `domain47_poison_prewarm_roots`) in isolation. It cannot prove that
// `build_compact_extraction_gate_records` CALLS them: deleting both
// production call sites leaves every one of those tests green. These two
// drive the real frame instead, so the call sites are what is under test.
// ---------------------------------------------------------------------

/// The d47 scheduled-domain name, as `compact_frame_domain_directly_enabled`
/// resolves it.
const RAW_D47_DOMAIN: &str = "characters_leaders_cabinet_live";
/// d82, which does NOT name d47 but pulls it in through
/// `OWNED_EMITTED_DOMAIN_DEPENDENCIES` (82 -> 47). Scheduling this and
/// nothing else is what tells a closure-derived enable guard apart from a
/// direct "was d47 named" check.
const RAW_D82_DOMAIN: &str = "character_presentation_live";

/// A state whose scheduler clock authenticates and reads exactly `hour`.
///
/// Nothing of domain 47's own world is staged, and that is deliberate: the
/// prewarm's clock bracket is decided before any of it is reached, so the
/// question "was this phase in the frame's hour" has an answer whether or not
/// there were any characters to find.
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

/// THE WIRING. A frame whose label is the hour the engine clock actually
/// reads must report `InFrameHour`; a frame whose label is one hour behind
/// the clock must report `WrongHour` and must poison d47's roots — and
/// nothing tore, so no bracket could have caught it.
///
/// This is the pair `d47_root_hour_tests` cannot express: the verdict here is
/// computed by the production frame builder from clock reads it took itself,
/// against the label it was handed.
///
/// MUTATION RUN: delete the `domain47_apply_prewarm_clock_verdict(...)` call
/// from `build_compact_extraction_gate_records`.
/// OBSERVED: `the frame must report the prewarm's clock verdict` — left 0
/// (`NotScheduled`, the field's default, i.e. nobody decided), right 1. All
/// nine `d47_root_hour_tests` stay green through the same mutation, which is
/// why this test exists.
#[test]
fn the_frame_builder_decides_the_d47_prewarm_clock_verdict_against_its_own_label() {
    let hour = fixture_game_hour_raw();
    let control = compact_control_scheduling(&[RAW_D47_DOMAIN]);

    // In-hour: the label is the hour the clock reads.
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

    // Wrong hour: the engine clock has already moved past the label. Both
    // clock reads agree, so no bracket TEARS -- but since
    // `HOUR_LABEL_MISMATCH_POLICY` became `Refuse`, an equal-but-wrong
    // bracket replaces the domain's records with an HourLabelMismatch gap,
    // so the verdict unit d47 computed never reaches the wire. The verdict
    // machinery itself is still pinned by the in-hour half above; what the
    // wrong-hour half now witnesses is the refusal.
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
    // Under `CountOnly` this test asserted the POISON (`skipped_root_rows`
    // nonzero): the mismatched frame was still published, so d47's own gate
    // was the only thing keeping wrong-hour roots off the wire. Under
    // `Refuse` the refusal itself runs `reset_trackers_after_rejected_bracket`,
    // which wipes the prewarm cache — poison included — so the cached
    // wrong-hour roots cannot survive into the NEXT frame either. A reset
    // cache is therefore the correct post-refusal state, and asserting it
    // pins the reset: skipping the reset would leave the poison marker (a
    // nonzero `skipped_root_rows`) behind.
    assert_eq!(
        wrong_hour
            .trackers
            .characters_leaders_cabinet_live
            .roots
            .skipped_root_rows,
        0,
        "the refusal must reset d47's prewarm cache; a surviving poison \
         marker means wrong-hour roots survived the refusal"
    );
    assert_eq!(
        wrong_hour
            .trackers
            .characters_leaders_cabinet_live
            .roots
            .stable_root_count(),
        0,
        "no root observed in the wrong hour may survive the refusal"
    );
    // Nothing tore, so the TEAR machinery cannot have produced the refusal
    // above. `CountOnly` is that machinery with the generalised hour-label
    // rule switched off, and it accepts this frame -- which is exactly the
    // hole d47's own refusal was closing before the rule was generalised.
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

/// THE OTHER CALL SITE. The prewarm must contribute its OWN clock bracket to
/// the frame, on top of the one d47's worker records — that is what makes a
/// straddled prewarm reach the wire as a Torn DOMAIN_GAP rather than as
/// nothing at all.
///
/// A straddle cannot be staged offline (it needs the engine clock to advance
/// mid-phase), so what is asserted is the bracket's EXISTENCE and its
/// observed hours, which is exactly what the missing call site would remove.
///
/// MUTATION RUN: delete the `lane_tracker.record_clock_bracket(...)` call for
/// the prewarm from `build_compact_extraction_gate_records`.
/// OBSERVED: `the prewarm must contribute a d47 clock bracket of its own`
/// — left 1, right 2. Every `d47_root_hour_tests` test stays green.
#[test]
fn the_prewarm_records_its_own_d47_clock_bracket_beside_the_workers() {
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
        2,
        "the prewarm must contribute a d47 clock bracket of its own, beside \
         the one d47's worker records"
    );
    for bracket in &d47_brackets {
        assert_eq!(
            bracket.opening.map(|(observed, _)| observed),
            Some(hour),
            "every d47 bracket observed the fixture's staged hour"
        );
        assert_eq!(bracket.closing.map(|(observed, _)| observed), Some(hour));
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
        "an in-hour frame must reject nothing, or the count above is of two \
         failures rather than two observations"
    );
}

/// THE ENABLE GUARD. Both tests above schedule d47 by name, so they cannot
/// tell the closure-derived `characters_leaders_cabinet_live_enabled` apart
/// from a direct "was d47 named" check. d47 is also reached as d82's declared
/// dependency (`OWNED_EMITTED_DOMAIN_DEPENDENCIES`, 82 -> 47), and on that
/// path the prewarm runs while a direct check would read false — so the
/// bracket and the verdict would be silently skipped for a frame that does
/// run the phase.
///
/// MUTATION RUN: guard both new call sites on
/// `control.scheduled_domains.iter().any(|name| name == "characters_leaders_cabinet_live")`
/// instead of `characters_leaders_cabinet_live_enabled`.
/// OBSERVED: 12 passed, 1 failed — this test, stopping at the assertion BEFORE
/// the bracket count: `a dependency-reached d47 must still have its prewarm
/// clock decided`, left Some(0) (`NotScheduled` — nobody decided), right
/// Some(1). Both tests above stay green, because they name d47 directly.
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

    // Premise: scheduling d82 alone really does run d47 in this frame. Without
    // this the assertions below would pass on a frame that never reached the
    // phase at all.
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
        d47_brackets, 2,
        "a d47 reached through d82's dependency closure still runs the \
         prewarm, so it still needs its bracket"
    );
}

/// THE ORDERING, which no offline runtime test can reach.
///
/// The three frame-driven tests above prove the call sites exist, that the
/// verdict is decided against the frame's label, and that a dependency-reached
/// d47 gets both. None can prove the two clock reads BRACKET the phase rather
/// than sitting beside it: staging a straddle needs the engine clock to
/// advance mid-prewarm, and the offline fixture's clock only moves when a test
/// writes it. Move both reads above the prewarm call and every runtime test in
/// this lane stays green while a straddle becomes structurally undetectable —
/// the bracket reads H/H, the verdict is `InFrameHour`, and mixed-hour roots
/// publish with no Torn rejection.
///
/// So the order is pinned in the source, the same way
/// `the_gate_function_binds_every_error_it_matches` pins its own claim about
/// the same function. Line based, so a reflow of unrelated code cannot decide
/// the verdict.
///
/// MUTATION RUN: moved `let prewarm_clock_closing = ...` to sit immediately
/// after `let prewarm_clock_opening = ...`, above the prewarm call.
/// OBSERVED: 12 passed, 1 failed — `the prewarm's closing clock read must come
/// AFTER the phase it closes (phase at line 40877, closing at line 40869)`.
/// All three frame-driven tests and all nine helper tests stay green.
#[test]
fn the_prewarm_clock_reads_bracket_the_phase_rather_than_sitting_beside_it() {
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
                // The comments above these statements name them on purpose; a
                // comment is not a call.
                !trimmed.starts_with("//") && trimmed.contains(needle)
            })
            .map(|offset| start + offset + 1)
            .unwrap_or_else(|| panic!("the gate function contains `{needle}`"))
    };
    let opening = find("let prewarm_clock_opening =");
    let phase = find("prewarm_compact_characters_leaders_cabinet_roots(");
    let closing = find("let prewarm_clock_closing =");
    assert!(
        opening < phase,
        "the prewarm's opening clock read must come BEFORE the phase it opens \
         (opening at line {opening}, phase at line {phase})"
    );
    assert!(
        phase < closing,
        "the prewarm's closing clock read must come AFTER the phase it closes \
         (phase at line {phase}, closing at line {closing})"
    );
    // And the verdict must be decided after BOTH reads exist, or it decides on
    // a `None` the closing read would have filled in.
    let verdict = find("domain47_apply_prewarm_clock_verdict(");
    assert!(
        closing < verdict,
        "the verdict must be applied after the closing read \
         (closing at line {closing}, verdict at line {verdict})"
    );
}

/// THE INSTRUMENT, minted by the real frame builder and read back off the wire.
///
/// `lw_hour_label_skew_abs` is ONE clock read taken at the top of the heavy
/// sweep; it answers a question about where the frame STARTED. These two
/// counters answer the question that decides the refusal's price: of the
/// brackets this frame actually held, how many read an hour that is not the
/// frame's label. Nothing on the wire could answer that before -- an accepted
/// bracket recorded its hour nowhere -- which is why t17 could only bound the
/// frame-level rate between 4.90% and 62.65% instead of stating it.
///
/// Driven through `build_compact_extraction_gate_records`, so it fails if the
/// counters are computed but never emitted, emitted but never computed, or
/// computed from the wrong brackets.
///
/// MUTATION RUN: pass `0, 0` in place of the two counters at the
/// `append_compact_lane_wall_probe_record` call site -- i.e. delete the call to
/// `hour_label_mismatched_bracket_count`.
/// OBSERVED: this test alone fails, on the premise assertion one line earlier
/// (`the frame probe must report how many brackets were held`), because
/// deleting the call deletes both counters at once. Every test in
/// `hour_label_tests` and every `d47_root_hour_tests` test stays green, because
/// none of them goes through the probe -- which is the whole reason this test
/// exists beside them.
///
/// MUTATION RUN (2): delete the `opening.0 != game_hour_raw` clause from
/// `bracket_hour_verdict`, i.e. the predicate rather than the wiring.
/// OBSERVED: `every bracket in this frame read an hour that is not the label`
/// — left `Some(0)`, right `Some(3)`.
#[test]
fn the_frame_probe_reports_how_many_brackets_missed_the_frames_label() {
    let hour = fixture_game_hour_raw();
    let control = compact_control_scheduling(&[RAW_D47_DOMAIN]);

    // In-hour: the label is the hour the staged clock reads.
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

    // Wrong hour: the staged clock has already moved past the label, and
    // nothing tears, so every bracket opens and closes in the SAME hour --
    // just not this frame's.
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
    // Both frames carry labels the calendar admits, so both must say the rule
    // APPLIED. Without this a zero mismatch count would be indistinguishable
    // from a frame the rule could not be run on at all.
    for (name, outcome) in [("in-hour", &in_hour), ("wrong-hour", &wrong_hour)] {
        assert_eq!(
            outcome.diagnostic_unit(RC_PROBE_DOMAIN_ID, "lw_hour_label_unusable"),
            Some(0),
            "a frame whose label the calendar admits must say the rule applied ({name})"
        );
    }
}

/// THE OTHER BRANCH. Both frames above carry labels the calendar admits, so
/// they only ever exercise `lw_hour_label_unusable == 0` -- a production edit
/// pinning the unit to the constant `0` passes every one of them.
///
/// A label the calendar refuses is not hypothetical: `0` is what the frame
/// builder substitutes when it had no authoritative clock, and a corrupt or
/// out-of-window scheduler stamp reaches the same place. The rule cannot be
/// applied to such a frame -- a gap could not encode against the label -- so
/// the frame must SAY so, or a zero mismatch count reads as "checked and
/// clean" when it means "not checkable".
///
/// MUTATION RUN: pin the unit's value to `0` in
/// `append_compact_lane_wall_probe_record`.
/// OBSERVED: `a frame the rule cannot be applied to must say so` — left
/// `Some(0)`, right `Some(1)`. Every other test in this file and in
/// `hour_label_tests` stays green, which is precisely the hole this closes.
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

// ---------------------------------------------------------------------
// Domain 43 -- market_trade_live's purchase-contract census
// ---------------------------------------------------------------------

/// Heap layout of [`market_trade_frame_fixture`]. Named rather than inline so
/// the two contract objects and their four sub-buffers cannot silently overlap
/// as the fixture grows.
mod d43_at {
    pub(super) const GAME_STATE: usize = 0x0_0000;
    pub(super) const MANAGER: usize = 0x0_1000;
    /// The two per-country outer tables, each `OUTER_COUNT` inline
    /// `{begin, cap, count}` headers at `MARKET_CONTRACT_VECTOR_STRIDE`.
    pub(super) const OUTER_TABLE_0: usize = 0x0_2000;
    pub(super) const OUTER_TABLE_1: usize = 0x0_2100;
    /// Pointer arrays of contract pointers, one per directional table.
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
    pub(super) const HEAP_LEN: usize = 0x1_0000;

    pub(super) const OUTER_COUNT: i32 = 2;
    pub(super) const CONTRACT_1_ID: u32 = 7001;
    pub(super) const CONTRACT_2_ID: u32 = 7002;
    pub(super) const EQUIPMENT_VARIANT_ID: u32 = 4242;
    /// Read back out of the delivery sub-buffer as the contract's amount.
    pub(super) const DELIVERY_AMOUNT_RAW: i32 = 250_000;
    pub(super) const DELIVERY_VARIANT_ID: u32 = 4243;
}

/// A world in which d43's rooted purchase-contract census SUCCEEDS: a
/// CGameState with a market manager, two countries whose
/// `CEquipmentMarketStatus` wrappers corroborate that manager, both
/// directional manager tables, and two purchase contracts each holding the
/// seller AND buyer membership the census requires.
///
/// Staged because the contract family is the one d43 family with no offline
/// coverage at all. Live captures t19/t20/t21 published it empty on every
/// frame — 294 records in t21, `observed_contract_rows = 0` — while the
/// resource families in the same payload published 439 countries' worth of
/// rows. With no fixture, "the walk is broken" and "the walk is fine and the
/// live world tripped one of its ~20 guards" were indistinguishable offline.
/// This fixture settles that half: it proves the walk and the emission chain
/// behind it produce rows when the world has the shape the census describes.
///
/// What it deliberately does NOT prove is the constants (staging writes
/// through the same `build_profile`/`offsets` values the reader then reads
/// through, so a wrong constant moves both sides together) — see this
/// module's header. The live failure is a question about the WORLD, and the
/// `d43_contract_failure_stage` unit this lane added is what answers it.
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
    // Also writes the label hour at `game_state + GS_CURRENT_DATE_OFFSET`;
    // without it every domain is refused under `HOUR_LABEL_MISMATCH_POLICY`.
    fixture.stage_scheduler_clock_at(at::GAME_STATE);

    let base = fixture.module.module_base();
    let country_vtable = base + p::DOMAIN57_COUNTRY_VTABLE_RVA;
    let contract_vtable = base + p::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    let variant_vtable = base + VARIANT_VTABLE_RVA;

    let manager = fixture.heap.at(at::MANAGER);
    fixture
        .heap
        .write_u64(at::GAME_STATE + CGAMESTATE_MARKET_MANAGER_OFF, manager as u64);

    // Two countries, each corroborating the global manager through its own
    // CEquipmentMarketStatus wrapper -- the check the census runs over every
    // country it is handed.
    for (country_at, status_at) in [
        (at::COUNTRY_1, at::MARKET_STATUS_1),
        (at::COUNTRY_2, at::MARKET_STATUS_2),
    ] {
        fixture.heap.write_u64(country_at, country_vtable as u64);
        let status = fixture.heap.at(status_at);
        fixture
            .heap
            .write_u64(country_at + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF, status as u64);
        fixture
            .heap
            .write_u64(status_at + CEQUIPMENT_MARKET_STATUS_MANAGER_OFF, manager as u64);
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

    // The two variant objects the sub-buffer scan is looking for.
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
    // A sub-buffer is scanned for the FIRST embedded pointer that resolves to
    // a type-70 variant; the amount is then taken from the 0x10 bytes that
    // follow it. Staging the pointer at offset 0 and the amount at +8 is the
    // shape that scan describes.
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

    // The contracts. Seller and buyer are distinct staged countries, and
    // `+ DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR` names the buyer's own object so
    // the row carries `BUYER_PTR_VALID` rather than a bare tag.
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
        fixture
            .heap
            .write_i32(contract_at + p::DOMAIN43_PURCHASE_CONTRACT_ID_ID, contract_id as i32);
        fixture
            .heap
            .write_i32(contract_at + p::DOMAIN43_PURCHASE_CONTRACT_SELLER_TAG_DB_ID, 1);
        fixture
            .heap
            .write_i32(contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_TAG_DB_ID, 2);
        fixture
            .heap
            .write_u64(contract_at + p::DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR, country_2 as u64);
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

    // Both contract pointers in BOTH directional tables, once each. The census
    // requires every contract to hold its seller AND buyer membership
    // (`market_contract_membership_is_complete`), so a one-sided staging would
    // be refused -- which is itself worth knowing, and is the negative control
    // below.
    let contract_1 = fixture.heap.at(at::CONTRACT_1);
    let contract_2 = fixture.heap.at(at::CONTRACT_2);
    for body_at in [at::INNER_BODY_0, at::INNER_BODY_1] {
        fixture.heap.write_u64(body_at, contract_1 as u64);
        fixture.heap.write_u64(body_at + 8, contract_2 as u64);
    }
    let inner_body_0 = fixture.heap.at(at::INNER_BODY_0);
    let inner_body_1 = fixture.heap.at(at::INNER_BODY_1);

    // Outer table 0 (manager + 0x30) carries the contracts in its slot 0;
    // outer table 1 (manager + 0x48) carries them in its slot 1. Which slot
    // holds them is immaterial to the walk -- it iterates `0..outer.count` and
    // never subscripts by tag -- but splitting them proves that.
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
    write_vector(&mut fixture.heap, at::OUTER_TABLE_1, 0, 0);
    write_vector(
        &mut fixture.heap,
        at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE,
        inner_body_1 as u64,
        2,
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

/// The headline d43 question, answered offline: with a market world staged,
/// does `market_trade_live` put purchase-contract ROWS on the wire?
///
/// Before this test the answer existed only in live capture, and there it was
/// "no" on every frame of t19, t20 and t21 with no attribution. The census
/// (`compact_market_trade_rooted_contract_ptrs`) had roughly twenty
/// `return None` sites and not one diagnostic, so an empty contract family and
/// a broken walk looked identical on the wire.
///
/// What production change makes this fail: any break in
/// `compact_market_trade_rooted_contract_ptrs`, `read_compact_market_trade_row`
/// or the d43 emission site. Mutation-proved; see the assertion comments and
/// the lane report.
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
    // THE regression this lane exists to hold. In t21 this was `0x300`.
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

    // The rows themselves, compared against what was STAGED rather than
    // against the reader's own output, so a reader that walks to the wrong
    // field cannot move both sides together.
    let state = fixture.collector_state();
    let country_by_tag = BTreeMap::from([
        (1u32, fixture.heap.at(d43_at::COUNTRY_1)),
        (2u32, fixture.heap.at(d43_at::COUNTRY_2)),
    ]);
    let vtable = state.base + compact_contract::build_profile::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA;
    let contract_ptrs =
        compact_market_trade_rooted_contract_ptrs(&state, vtable, &country_by_tag)
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

    // Those rows reached the wire: the DOMAIN_DATA payload's contract block is
    // the production encoding of them. The header's `row_count` is read back
    // off the emitted bytes rather than from the perf record.
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

    // This frame IS partial -- the fixture stages the market world only, so
    // the resource, flow, factory, classic-deal and stockpile families have no
    // roots and each badge PARTIAL_DATA. That is what makes the assertion
    // below worth making: the diagnostics ARE published, and the contract unit
    // must state its own success rather than inherit the frame's partiality or
    // hold a stale stage from an earlier frame in the same process.
    assert_ne!(
        perf.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "the market-only fixture leaves d43's other families absent"
    );
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_failure_stage"),
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
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_MARKET_TRADE_LIVE);
}

/// The negative control, and the reason the instrumentation this lane added
/// has a job: take the SAME world and remove one contract's membership from
/// the second directional table. The census refuses the whole census — which
/// is the t19/t20/t21 wire exactly — and the new
/// `d43_contract_failure_stage` unit is what distinguishes this refusal from
/// the other nineteen.
///
/// What production change makes this fail: deleting any `*stage = ...` write
/// in `compact_market_trade_rooted_contract_ptrs_staged`, or reverting the
/// census to the uninstrumented form, which would leave the unit at `0` and
/// name no site. Mutation-proved; see the lane report.
#[test]
fn a_one_sided_purchase_contract_membership_names_the_stage_that_refused_it() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    // Contract 2 keeps its table-0 membership and loses its table-1 one, so
    // its membership word is `0b01` rather than `0b11`. The vector is
    // shortened rather than zeroed: a null pointer would be refused earlier,
    // at the contract-identity stage, and would prove a different thing.
    fixture
        .heap
        .write_i32(d43_at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE + MARKET_VECTOR_COUNT_OFF, 1);
    fixture
        .heap
        .write_i32(d43_at::OUTER_TABLE_1 + MARKET_CONTRACT_VECTOR_STRIDE + MARKET_VECTOR_CAP_OFF, 1);

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
    // The whole point: the wire now names WHICH guard refused. Without the
    // instrumentation this assertion has no unit to read and the two failures
    // -- this one and a missing market-status wrapper -- are the same bytes.
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_failure_stage"),
        Some(u64::from(domain43_contract_stage::MEMBERSHIP_INCOMPLETE)),
        "the incomplete-membership refusal must name its own stage"
    );

    // And the width evidence is published on the failing frame too, so one
    // capture can rule the width hypothesis in or out without a second.
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_scope_countries"),
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

/// The second negative control, aimed squarely at the standing hypothesis for
/// the live failure: a country whose `CEquipmentMarketStatus` wrapper is
/// absent.
///
/// The corroboration loop is all-or-nothing over the country set the census is
/// handed, and in production that set is WIDER than the one every family that
/// succeeded in t21 walks — it is the active-country snapshot plus reserved
/// dynamic slots (`extend_market_trade_country_ptrs_with_dynamic_slots`). One
/// such slot with no wrapper would produce exactly the measured wire. This
/// test pins the SHAPE of that failure offline so a live capture reporting
/// stage 202 is immediately conclusive rather than merely suggestive.
///
/// What production change makes this fail: removing the wrapper guard, or
/// merging stages 201/202/203 back into one code. Mutation-proved; see the
/// lane report.
#[test]
fn a_country_without_a_market_status_wrapper_refuses_the_whole_contract_census() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = market_trade_frame_fixture();
    // Country 2 keeps every other field and loses only its wrapper pointer --
    // the shape a reserved dynamic country slot takes.
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
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_contract_failure_stage"),
        Some(u64::from(
            domain43_contract_stage::CORROBORATE_WRAPPER_NOT_HEAP
        )),
        "the refusal names the corroboration stage, not one of the other nineteen"
    );
    // And it names WHICH country, which is what turns a live stage-202 report
    // into an actionable one.
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_contract_failure_tag_db_id"
        ),
        Some(2),
        "the offending country accompanies the stage"
    );
}

