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

    /// Hold this file's PDU1 decoder to the production one.
    ///
    /// `compact_perf_diagnostic_unit_names` is the shipped reader of the
    /// same trailer; it emits `name=value` for NON-ZERO units only, up to
    /// six. Every non-zero unit this fixture decoded for `domain_id` must
    /// appear there in the same order, so the two decoders cannot disagree
    /// about the part they both cover.
    pub(super) fn assert_diagnostics_agree_with_production(&self, domain_id: u16) {
        let record = self
            .record(domain_id, COMPACT_RECORD_TYPE_DOMAIN_PERF)
            .unwrap_or_else(|| panic!("domain {domain_id} emitted a perf record"));
        let mine = DomainPerfView::parse(&record.payload)
            .diagnostics
            .into_iter()
            .filter(|(_, value)| *value != 0)
            .map(|(name, value)| format!("{name}={value}"))
            .take(6)
            .collect::<Vec<_>>();
        let production = compact_perf_diagnostic_unit_names(&record.payload);
        assert_eq!(
            mine, production,
            "the fixture's PDU1 decode must agree with the production reader              on domain {domain_id}"
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
    let (arena, mut state) = compact_naval_fixture_at_module_base(image.module_base());
    bind_scheduler_clock(&mut image, &mut state);
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

