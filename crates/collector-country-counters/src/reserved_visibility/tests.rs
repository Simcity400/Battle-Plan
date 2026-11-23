use super::*;
use engine_native::{cgame_state, crelation::{self, subject as csubject_relation}};
use engine_native_memory::{memory::SliceMemory, walk::Outcome};
use std::cell::Cell as Counter;

const STATE: usize = 0x1000;
const RESERVED: usize = 0x4000;
const RESERVED_DIPLOMACY: usize = 0x7000;
const RESERVED_MATRIX: usize = 0x8000;
const POSITIVE: usize = 0x10000;
const POSITIVE_DIPLOMACY: usize = 0x14000;
const POSITIVE_MATRIX: usize = 0x15000;
const WAR: usize = 0x19000;
const SUBJECT: usize = 0x19100;

fn status(slot: usize) -> usize {
    0x9000 + slot * 0x800
}
fn forward_status(slot: usize) -> usize {
    0x16000 + slot * 0x800
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data);
    put32(bytes, at + 8, count);
    put32(bytes, at + 12, count);
}
fn world(slots: u32) -> Vec<u8> {
    let mut bytes = vec![0; 0x20000];
    header(&mut bytes, STATE + cgame_state::COUNTRIES, 0x3000, 2);
    put64(&mut bytes, 0x3000, RESERVED);
    put64(&mut bytes, 0x3008, POSITIVE);
    header(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x3100, 3);
    put32(&mut bytes, 0x3104, 1);
    for (country, id, root, matrix) in [
        (RESERVED, 0, RESERVED_DIPLOMACY, RESERVED_MATRIX),
        (POSITIVE, 1, POSITIVE_DIPLOMACY, POSITIVE_MATRIX),
    ] {
        put64(&mut bytes, country, ccountry::VTABLE_RVA.0);
        put32(&mut bytes, country + ccountry::ID, id);
        put64(&mut bytes, country + ccountry::DIPLOMACY_STATUS, root);
        put64(&mut bytes, root, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        put64(&mut bytes, root + engine_native_parts::cdiplomacy_status_layout::COUNTRY, country);
        header(&mut bytes, root + engine_native_parts::cdiplomacy_status_layout::RELATIONS, matrix, slots);
        for slot in 0..slots as usize {
            let cell = if id == 0 {
                status(slot)
            } else {
                forward_status(slot)
            };
            put64(&mut bytes, matrix + slot * 8, cell);
            put64(&mut bytes, cell, crelation_status::VTABLE_RVA.0);
            put64(&mut bytes, cell + crelation_status::DIPLOMACY_STATUS, root);
            put32(
                &mut bytes,
                cell + crelation_status::TARGET_COUNTRY,
                slot as u32,
            );
        }
    }
    bytes
}

struct MemoryProbe<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    attempts: Counter<usize>,
    body_reads: Counter<usize>,
    prefix_seen: Counter<bool>,
    require_prefix: bool,
}
impl Memory for MemoryProbe<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == POSITIVE + ccountry::PRODUCTION_STATUS {
            self.prefix_seen.set(true);
        }
        if address == RESERVED_DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS && self.require_prefix {
            assert!(
                self.prefix_seen.get(),
                "reverse header preceded an established positive prefix"
            );
        }
        if (RESERVED_MATRIX..RESERVED_MATRIX + 32).contains(&address)
            || (status(0)..status(4)).contains(&address)
        {
            self.body_reads.set(self.body_reads.get() + 1);
        }
        if self.fault == Some(address) {
            let count = self.attempts.get() + 1;
            self.attempts.set(count);
            if count == 1 {
                return false;
            }
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine_native_memory::country_identity::Country, &'static str>> {
        (base == 0 && state == STATE).then_some(Ok(engine_native_memory::country_identity::Country {
            pointer: RESERVED,
            canonical_slot: 0,
            id: 0,
        }))
    }
}
fn memory(bytes: &[u8], fault: Option<usize>) -> MemoryProbe<'_> {
    MemoryProbe {
        bytes,
        fault,
        attempts: 0.into(),
        body_reads: 0.into(),
        prefix_seen: false.into(),
        require_prefix: false,
    }
}
fn observation(memory: &impl Memory) -> ccountry::ReservedVisibilityObservation {
    let mut naming = ccountry::read_reserved_naming_observed_inner(memory, 0, STATE, engine_native_memory::walk::Buffer::Count, None);
    let observed = naming
        .visibility
        .take()
        .expect("completed controls survive naming refusal");
    naming.launch_states = None;
    observed
}

#[test]
fn short_matrix_grows_in_the_pass_and_failed_prefix_skips_body_and_clears_previous_cells() {
    let bytes = world(3);
    let memory = memory(&bytes, None);
    let observed = observation(&memory);
    let mut capture = Captured::new(1);
    let source = capture
        .prepare(&memory, Some(51), Some(observed), true)
        .unwrap();
    assert_eq!(capture.cells.required, 3);
    assert!(capture.cells.first.len() >= 3);
    capture.capture_cell(&source, &memory, 0, None, 0, None);
    assert!(memory.body_reads.get() > 0);
    let owned = capture.projection().unwrap().unwrap();
    assert_eq!(owned.cells.len(), 3);
    assert!(owned.cells[0].is_some());
    let old_reads = memory.body_reads.get();
    drop(source);
    let source = capture
        .prepare(&memory, Some(53), Some(observed), false)
        .unwrap();
    capture.capture_cell(&source, &memory, 0, None, 0, None);
    assert_eq!(memory.body_reads.get(), old_reads);
    assert!(capture.cells.first.iter().all(|cell| cell.value.is_none()));
    assert!(capture.projection().unwrap().is_none());
    assert_eq!(owned.sample_hour, replay_data::rows::PacketHour::Host);
    assert!(
        owned.cells[0].is_some(),
        "off-thread projection owns its values"
    );
}

#[test]
fn same_pointer_forward_failure_is_not_retried() {
    let mut bytes = world(2);
    put64(
        &mut bytes,
        forward_status(0) + crelation_status::WAR_RELATION,
        WAR,
    );
    put64(&mut bytes, status(1) + crelation_status::WAR_RELATION, WAR);
    bytes[WAR + crelation::RETIRED] = 1;
    let memory = memory(&bytes, Some(WAR + crelation::RETIRED));
    let observed = observation(&memory);
    let countries = ccountry::Reader::new(&memory, 0, STATE).unwrap();
    let country = countries.resolve(1).unwrap();
    let mut forward = crelation_status::ForwardZeroObservation::default();
    let inputs = crelation_status::co_belligerent_inputs_observed_buffer(
        &memory,
        0,
        &countries,
        country,
        engine_native_memory::walk::Buffer::Growing(&mut vec![0; 2]),
        &mut forward,
    );
    assert!(inputs.zero_war.is_err());
    let mut capture = Captured::new(2);
    let source = capture
        .prepare(&memory, Some(71), Some(observed), true)
        .unwrap();
    capture.capture_cell(&source, &memory, 0, Some(&countries), 0, None);
    capture.capture_cell(&source, &memory, 0, Some(&countries), 1, Some(&forward));
    assert_eq!(memory.attempts.get(), 1);
    let packet = capture.projection().unwrap().unwrap();
    assert_eq!(packet.cells[0].unwrap().war, Some([0, 0]));
    assert_eq!(packet.cells[1].unwrap().war, None);
}

#[test]
fn cell_status_fault_is_local_and_partial_subject_preserves_order_and_fault() {
    let mut bytes = world(3);
    put64(
        &mut bytes,
        status(0) + crelation_status::SUBJECT_RELATION,
        SUBJECT,
    );
    put64(&mut bytes, SUBJECT, csubject_relation::VTABLE_RVA.0);
    put32(
        &mut bytes,
        SUBJECT + crelation::TYPE_ID,
        csubject_relation::TYPE_VALUE,
    );
    put32(&mut bytes, SUBJECT + crelation::FIRST_COUNTRY, 1);
    put32(&mut bytes, SUBJECT + crelation::SECOND_COUNTRY, 0);
    put64(&mut bytes, status(1), 0);
    let memory = memory(&bytes, Some(SUBJECT + crelation::FIRST_COUNTRY));
    let observed = observation(&memory);
    let mut capture = Captured::new(3);
    let source = capture
        .prepare(&memory, Some(81), Some(observed), true)
        .unwrap();
    capture.capture_cell(&source, &memory, 0, None, 0, None);
    capture.capture_cell(&source, &memory, 0, None, 1, None);
    let packet = capture.projection().unwrap().unwrap();
    assert_eq!(
        packet.cells[0].unwrap().subject.unwrap().parties,
        Some([None, Some(0)])
    );
    assert!(packet.cells[1].is_none());
    assert!(
        packet.cells[2].is_none(),
        "unreached slots stay uncertified"
    );
    assert_eq!(memory.attempts.get(), 1);
}

#[test]
fn one_reader_supports_zero_alias_even_with_no_positive_prefix_and_raw_zero_survives_reader_fault()
{
    let mut bytes = world(2);
    put32(&mut bytes, status(0) + crelation_status::TARGET_COUNTRY, 2);
    let memory = memory(&bytes, None);
    let observed = observation(&memory);
    let mut counters = super::super::country_counters::Captured::vacant();
    let mut capture = Captured::new(2);
    counters.capture_with_reverse(
        &memory,
        0,
        STATE,
        Some(Request {
            hour: Some(91),
            observation: Some(observed),
            prefix_ready: true,
            capture: &mut capture,
        }),
    );
    assert_eq!(
        capture.projection().unwrap().unwrap().cells[0]
            .unwrap()
            .target,
        Some(2)
    );
    put32(&mut bytes, status(0) + crelation_status::TARGET_COUNTRY, 0);
    let memory = self::memory(&bytes, Some(STATE + cgame_state::COUNTRIES));
    let observed = observation(&memory);
    counters.capture_with_reverse(
        &memory,
        0,
        STATE,
        Some(Request {
            hour: Some(92),
            observation: Some(observed),
            prefix_ready: true,
            capture: &mut capture,
        }),
    );
    assert_eq!(
        capture.projection().unwrap().unwrap().cells[0]
            .unwrap()
            .target,
        Some(0)
    );
}

#[test]
fn integration_starts_after_all_positive_prefixes_and_successive_growth_uses_live_matrix() {
    let bytes = world(2);
    let mut memory = memory(&bytes, None);
    memory.require_prefix = true;
    let observed = observation(&memory);
    let mut counters =
        super::super::country_counters::Captured::with_capacities(&Default::default());
    let mut capture = Captured::new(2);
    counters.capture_with_reverse(
        &memory,
        0,
        STATE,
        Some(Request {
            hour: Some(101),
            observation: Some(observed),
            prefix_ready: true,
            capture: &mut capture,
        }),
    );
    assert!(memory.prefix_seen.get());
    let mut capacity = 2;
    capture.grow(&mut capacity);
    let retained = capacity;
    assert_eq!(capture.cells.first.len(), retained);
    let next_slots = retained.checked_add(1).unwrap();
    let bytes = world(u32::try_from(next_slots).unwrap());
    let memory = self::memory(&bytes, None);
    let source = capture
        .prepare(&memory, Some(102), Some(observation(&memory)), true)
        .unwrap();
    assert_eq!(capture.cells.required, next_slots);
    assert!(capture.cells.first.len() >= next_slots);
    let body_reads = memory.body_reads.get();
    capture.capture_cell(&source, &memory, 0, None, 0, None);
    assert!(capture.refusal().is_none());
    let packet = capture.projection().unwrap().unwrap();
    assert_eq!(packet.cells.len(), next_slots);
    assert_eq!(packet.cells[0].unwrap().target, Some(0));
    assert!(
        packet.cells[1..].iter().all(Option::is_none),
        "unreached slots stay uncertified"
    );
    assert!(
        memory.body_reads.get() > body_reads,
        "the grown pass reaches the body"
    );
    drop(source);
    capture.grow(&mut capacity);
    assert!(capacity >= next_slots && capacity >= retained);
    let before = capture.allocated_bytes();
    capture.reset();
    capture.grow(&mut capacity);
    assert_eq!(capture.allocated_bytes(), before);
}

#[test]
fn failed_original_prefix_header_is_not_retried_for_a_positive_alias_to_zero() {
    let mut bytes = world(2);
    put32(&mut bytes, status(0) + crelation_status::TARGET_COUNTRY, 2);
    let baseline_memory = memory(&bytes, Some(STATE + cgame_state::COUNTRIES));
    let mut baseline = super::super::country_counters::Captured::vacant();
    baseline.capture_with_reverse(&baseline_memory, 0, STATE, None);
    let memory = memory(&bytes, Some(STATE + cgame_state::COUNTRIES));
    let observed = observation(&memory);
    let mut counters = super::super::country_counters::Captured::vacant();
    let mut capture = Captured::new(2);
    counters.capture_with_reverse(
        &memory,
        0,
        STATE,
        Some(Request {
            hour: Some(111),
            observation: Some(observed),
            prefix_ready: true,
            capture: &mut capture,
        }),
    );
    assert_eq!(
        memory.attempts.get(),
        baseline_memory.attempts.get(),
        "reverse adds no retry to the original failed header"
    );
    assert_eq!(
        counters.outcomes()[0],
        Outcome::Unavailable {
            reason: "country array unreadable or invalid"
        }
    );
    let cell = capture.projection().unwrap().unwrap().cells[0].unwrap();
    assert_eq!(cell.war, Some([0, 0]));
    assert_eq!(
        cell.target, None,
        "an unknown positive link is not fabricated as zero"
    );
}

#[test]
fn early_scalar_fault_survives_naming_refusal() {
    let bytes = world(2);
    for (address, reason) in [
        (
            RESERVED_DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::FACTION,
            "reserved faction pointer unreadable",
        ),
        (
            RESERVED_DIPLOMACY,
            "reserved naming object identity mismatch",
        ),
    ] {
        let memory = memory(&bytes, Some(address));
        let observed = observation(&memory);
        let mut capture = Captured::new(2);
        let source = capture.prepare(&memory, Some(121), Some(observed), false);
        assert!(capture.projection().unwrap().is_none());
        assert_eq!(capture.scalar_reason, Some(reason));
        assert_eq!(
            memory.attempts.get(),
            1,
            "scalar diagnostics must not retry"
        );
        drop(source);
    }
}

#[test]
fn reverse_military_field_fault_withholds_no_other_cell_input() {
    let bytes = world(2);
    let memory = memory(&bytes, Some(status(0) + crelation_status::MILITARY_ACCESS));
    let observed = observation(&memory);
    let mut capture = Captured::new(2);
    let source = capture
        .prepare(&memory, Some(131), Some(observed), true)
        .unwrap();
    capture.capture_cell(&source, &memory, 0, None, 0, None);
    let native = capture.cells.first[0].value.unwrap();
    assert_eq!(native.military_access, None);
    assert_eq!(native.war, Some([0, 0]));
    assert_eq!(native.target, Some(0));
    assert_eq!(memory.attempts.get(), 1);
}

#[test]
fn original_reserved_matrix_grows_before_first_or_later_cell_publication() {
    let mut bytes = world(2);
    let mut capture = Captured::new(1);
    for slots in [5_001, 20_001] {
        header(
            &mut bytes,
            RESERVED_DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            RESERVED_MATRIX,
            slots,
        );
        let memory = memory(&bytes, None);
        let observed = observation(&memory);
        let source = capture
            .prepare(&memory, Some(151), Some(observed), true)
            .unwrap();
        assert_eq!(capture.cells.required, slots as usize);
        assert_eq!(capture.cells.len, slots as usize);
        assert!(capture.cells.first.len() >= slots as usize);
        assert_eq!(memory.body_reads.get(), 0);
        capture.capture_cell(&source, &memory, 0, None, 0, None);
        assert!(capture.refusal().is_none());
        assert!(capture.cells.first[0].value.is_some());
    }
}
