use super::*;
use engine_native_parts::creference_registry::test_support::Fixture;
use std::cell::{Cell, RefCell};

const TASK: usize = 0x5000;
const UNIT: usize = 0x7000;
const REFS: usize = 0x8000;
const STRINGS: usize = 0x9000;
const REFERENCE: u64 = (42_u64 << 32) | 100;

struct Observed {
    memory: Fixture,
    reads: RefCell<Vec<(usize, usize)>>,
    fail_once: Cell<Option<usize>>,
}

impl Memory for Observed {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_once.get() == Some(address) {
            self.fail_once.set(None);
            output.fill(0xee);
            return false;
        }
        self.memory.copy(address, output)
    }
}

fn array(memory: &mut Fixture, at: usize, data: usize, count: u32) {
    memory.put(at, &(data as u64).to_le_bytes());
    memory.put(at + 8, &count.to_le_bytes());
    memory.put(at + 12, &count.to_le_bytes());
}

fn world(class: u32, owner: u32, searching: &[u64]) -> Observed {
    let mut memory = Fixture::new(100);
    memory.entry(
        Fixture::index(REFERENCE),
        1,
        REFERENCE,
        (UNIT + 0x10) as u64,
    );
    let (primary, secondary) = match class {
        0 => (
            crate::carmy::VTABLE_RVA.0,
            crate::carmy::SECONDARY_VTABLE_RVA.0,
        ),
        1 => (
            super::super::VTABLE_RVA.0,
            super::super::SECONDARY_VTABLE_RVA.0,
        ),
        13 => (
            engine_native_parts::crailway_gun::VTABLE_RVA.0,
            engine_native_parts::crailway_gun::SECONDARY_VTABLE_RVA.0,
        ),
        _ => (0, 0),
    };
    let mut header = [0; 32];
    header[..8].copy_from_slice(&(primary as u64).to_le_bytes());
    header[8..12].copy_from_slice(&class.to_le_bytes());
    header[16..24].copy_from_slice(&(secondary as u64).to_le_bytes());
    header[24..32].copy_from_slice(&REFERENCE.to_le_bytes());
    memory.put(UNIT, &header);
    memory.put(UNIT + cunit::ROSTER_OWNER, &owner.to_le_bytes());
    memory.put(UNIT + cunit::LOGICAL_OWNER, &999_u32.to_le_bytes());
    array(&mut memory, TASK + SEARCHING, REFS, searching.len() as u32);
    for (index, reference) in searching.iter().enumerate() {
        memory.put(REFS + index * 8, &reference.to_le_bytes());
    }
    memory.put(TASK + cunit::COMBAT_REFERENCE_COUNT, &0_i32.to_le_bytes());
    memory.put(TASK + ENGAGEMENT_TARGET, &0_u64.to_le_bytes());
    memory.put(
        TASK + super::super::NAVAL_MISSION,
        &(engine_native_parts::cnaval_mission::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    memory.put(
        TASK + super::super::NAVAL_MISSION + 8,
        &(TASK as u64).to_le_bytes(),
    );
    memory.put(TASK + super::super::DETACHED_ACTIVITY, &0_i32.to_le_bytes());
    memory.put(
        TASK + super::super::NAVAL_MISSION + engine_native_parts::cnaval_mission::ENGAGEMENT_RULE,
        &2_i32.to_le_bytes(),
    );
    array(&mut memory, TASK + AVOIDED_SHIP_TYPES, STRINGS, 0);
    Observed {
        memory,
        reads: RefCell::new(Vec::new()),
        fail_once: Cell::new(None),
    }
}

fn strings(memory: &mut Fixture, values: &[&[u8]]) {
    array(
        memory,
        TASK + AVOIDED_SHIP_TYPES,
        STRINGS,
        values.len() as u32,
    );
    for (index, value) in values.iter().enumerate() {
        assert!(value.len() <= 15);
        let mut header = [0; 32];
        header[..value.len()].copy_from_slice(value);
        header[16..24].copy_from_slice(&(value.len() as u64).to_le_bytes());
        header[24..].copy_from_slice(&15_u64.to_le_bytes());
        memory.put(STRINGS + index * 32, &header);
    }
}

fn prefix() -> super::super::CounterRow {
    super::super::CounterRow {
        id: REFERENCE,
        engagement_rule: 2,
        ..Default::default()
    }
}

#[test]
fn duplicate_searchers_reuse_exact_full_reference_and_roster_owner_for_generic_classes() {
    for (class, owner) in [(0, 0), (1, 17), (13, u32::MAX)] {
        let memory = world(class, owner, &[REFERENCE, REFERENCE, 0]);
        let (mut searchers, mut avoided, mut bytes, mut answers) = (
            vec![Searcher::default(); 3],
            Vec::new(),
            Vec::new(),
            vec![ReferenceAnswer::default(); 8],
        );
        let mut capture = Collector::new(
            &memory,
            0,
            123,
            &mut searchers,
            &mut avoided,
            &mut bytes,
            &mut answers,
        );
        let input = capture.capture(TASK, 0, &prefix());
        let counts = capture.finish();
        assert_eq!(input.sample_hour, 123);
        assert_eq!(input.searching, Some((0, 3)));
        assert_eq!(counts.searching_len, 3);
        assert_eq!(
            searchers[0],
            Searcher {
                reference: Some(REFERENCE),
                resolves: Some(true),
                roster_owner: Some(owner)
            }
        );
        assert_eq!(searchers[1], searchers[0]);
        assert_eq!(
            searchers[2],
            Searcher {
                reference: Some(0),
                resolves: Some(false),
                roster_owner: None
            }
        );
        let reads = memory.reads.borrow();
        assert_eq!(reads.iter().filter(|&&(at, _)| at == UNIT).count(), 1);
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, _)| at == UNIT + cunit::ROSTER_OWNER)
                .count(),
            1
        );
        assert!(
            !reads
                .iter()
                .any(|&(at, _)| at == UNIT + cunit::LOGICAL_OWNER)
        );
        assert!(
            answers
                .iter()
                .all(|answer| answer.resolution.is_none() && answer.owner.is_none())
        );
    }
}

#[test]
fn late_partial_raw_slot_keeps_the_known_first_match_prefix_and_never_retries() {
    let memory = world(1, 17, &[REFERENCE, REFERENCE, REFERENCE]);
    memory.fail_once.set(Some(REFS + 8));
    let (mut searchers, mut avoided, mut bytes, mut answers) = (
        vec![Searcher::default(); 3],
        Vec::new(),
        Vec::new(),
        vec![ReferenceAnswer::default(); 8],
    );
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    assert_eq!(capture.capture(TASK, 0, &prefix()).searching, Some((0, 3)));
    capture.finish();
    assert_eq!(searchers[0].roster_owner, Some(17));
    assert_eq!(searchers[1], Searcher::default());
    assert_eq!(searchers[2], Searcher::default());
    let reads = memory.reads.borrow();
    assert_eq!(reads.iter().filter(|&&(at, _)| at == REFS + 8).count(), 1);
    assert!(!reads.iter().any(|&(at, _)| at == REFS + 16));
}

#[test]
fn bucket_fault_is_sticky_across_duplicate_references_and_target_liveness() {
    let mut memory = world(1, 17, &[REFERENCE, REFERENCE]);
    memory
        .memory
        .put(TASK + ENGAGEMENT_TARGET, &REFERENCE.to_le_bytes());
    let bucket = 0x2000 + Fixture::index(REFERENCE) * engine_native_parts::reference_bucket::STRIDE;
    memory.fail_once.set(Some(bucket));
    let (mut searchers, mut avoided, mut bytes, mut answers) = (
        vec![Searcher::default(); 2],
        Vec::new(),
        Vec::new(),
        vec![ReferenceAnswer::default(); 8],
    );
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    let input = capture.capture(TASK, 2, &prefix());
    capture.finish();
    assert_eq!(searchers[0].resolves, None);
    assert_eq!(searchers[1].resolves, None);
    assert_eq!(
        input.engagement_target,
        Some(EngagementTarget {
            reference: REFERENCE,
            resolves: None
        })
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == bucket)
            .count(),
        1
    );
}

#[test]
fn bad_searcher_identity_withholds_owner_but_cannot_change_native_target_liveness() {
    for offset in [0, 16, 24] {
        let mut memory = world(1, 17, &[REFERENCE]);
        memory.memory.put(UNIT + offset, &0_u64.to_le_bytes());
        memory
            .memory
            .put(TASK + ENGAGEMENT_TARGET, &REFERENCE.to_le_bytes());
        let (mut searchers, mut avoided, mut bytes, mut answers) = (
            vec![Searcher::default(); 1],
            Vec::new(),
            Vec::new(),
            vec![ReferenceAnswer::default(); 4],
        );
        let mut capture = Collector::new(
            &memory,
            0,
            1,
            &mut searchers,
            &mut avoided,
            &mut bytes,
            &mut answers,
        );
        let input = capture.capture(TASK, 2, &prefix());
        capture.finish();
        assert_eq!(searchers[0].resolves, None);
        assert_eq!(searchers[0].roster_owner, None);
        assert_eq!(input.engagement_target.unwrap().resolves, Some(true));
        assert_eq!(input.avoided_ship_types, None);
    }
}

#[test]
fn authenticated_owner_word_fault_keeps_known_resolution_and_is_not_retried() {
    let mut memory = world(1, 17, &[REFERENCE, REFERENCE]);
    memory
        .memory
        .put(TASK + ENGAGEMENT_TARGET, &REFERENCE.to_le_bytes());
    memory.fail_once.set(Some(UNIT + cunit::ROSTER_OWNER));
    let (mut searchers, mut avoided, mut bytes, mut answers) = (
        vec![Searcher::default(); 2],
        Vec::new(),
        Vec::new(),
        vec![ReferenceAnswer::default(); 8],
    );
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    let input = capture.capture(TASK, 2, &prefix());
    capture.finish();
    assert_eq!(searchers[0].resolves, Some(true));
    assert_eq!(searchers[0].roster_owner, None);
    assert_eq!(searchers[1], searchers[0]);
    assert_eq!(input.engagement_target.unwrap().resolves, Some(true));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == UNIT + cunit::ROSTER_OWNER)
            .count(),
        1
    );
}

#[test]
fn a_zero_id_refuses_generic_identity_without_adding_a_target_liveness_gate() {
    let mut memory = world(1, 17, &[100]);
    memory
        .memory
        .entry(Fixture::index(100), 1, 100, (UNIT + 0x10) as u64);
    memory
        .memory
        .put(UNIT + cunit::HANDLE, &100_u64.to_le_bytes());
    memory
        .memory
        .put(TASK + ENGAGEMENT_TARGET, &100_u64.to_le_bytes());
    let (mut searchers, mut avoided, mut bytes, mut answers) = (
        vec![Searcher::default(); 1],
        Vec::new(),
        Vec::new(),
        vec![ReferenceAnswer::default(); 4],
    );
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    let input = capture.capture(TASK, 2, &prefix());
    capture.finish();
    assert_eq!(searchers[0].reference, Some(100));
    assert_eq!(searchers[0].resolves, None);
    assert_eq!(input.engagement_target.unwrap().resolves, Some(true));
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == UNIT + cunit::ROSTER_OWNER)
    );
}

#[test]
fn target_liveness_has_no_kind_gate_and_dangling_values_keep_the_raw_handle() {
    for (pointer, expected) in [(0, false), (0x10, false), (0xa000, true)] {
        let mut memory = world(1, 17, &[]);
        memory
            .memory
            .entry(Fixture::index(REFERENCE), 1, REFERENCE, pointer);
        memory
            .memory
            .put(TASK + ENGAGEMENT_TARGET, &REFERENCE.to_le_bytes());
        let (mut searchers, mut avoided, mut bytes, mut answers) =
            (Vec::new(), Vec::new(), Vec::new(), vec![ReferenceAnswer::default(); 2]);
        let mut capture = Collector::new(
            &memory,
            0,
            1,
            &mut searchers,
            &mut avoided,
            &mut bytes,
            &mut answers,
        );
        let input = capture.capture(TASK, 2, &prefix());
        capture.finish();
        assert_eq!(
            input.engagement_target,
            Some(EngagementTarget {
                reference: REFERENCE,
                resolves: Some(expected)
            })
        );
        assert!(!memory.reads.borrow().iter().any(|&(at, _)| at == 0xa000));
        assert_eq!(input.avoided_ship_types, (!expected).then_some((0, 0)));
    }
}

#[test]
fn exact_nullable_string_slots_preserve_raw_bytes_empty_and_late_faults() {
    let mut memory = world(1, 17, &[]);
    strings(&mut memory.memory, &[&[0xff, 0, 7], b"", b"bad", b"tail"]);
    memory
        .memory
        .put(STRINGS + 2 * 32 + 16, &99_u64.to_le_bytes());
    let (mut searchers, mut avoided, mut bytes, mut answers) =
        (Vec::new(), vec![None; 4], vec![0; 20], vec![ReferenceAnswer::default(); 2]);
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    let input = capture.capture(TASK, 2, &prefix());
    let counts = capture.finish();
    assert_eq!(input.avoided_ship_types, Some((0, 4)));
    assert_eq!(avoided, [Some((0, 3)), Some((3, 0)), None, Some((3, 4))]);
    assert_eq!(
        &bytes[..counts.bytes_len],
        &[0xff, 0, 7, b't', b'a', b'i', b'l']
    );
    assert_eq!(counts.bytes_required, 7);
}

#[test]
fn early_status_gates_and_rule_four_leave_late_groups_unconsumed() {
    for (mission, detached, combats, rule, has_target) in [
        (0_u32, 0_i32, 0_i32, 2_i32, false),
        (2, 1, 0, 2, false),
        (2, 0, 1, 2, false),
        (2, 0, 0, 4, true),
    ] {
        let mut memory = world(1, 17, &[]);
        memory
            .memory
            .put(TASK + cunit::COMBAT_REFERENCE_COUNT, &combats.to_le_bytes());
        let mut prefix = prefix();
        prefix.detached_activity = detached;
        prefix.engagement_rule = rule;
        let (mut searchers, mut avoided, mut bytes, mut answers) =
            (Vec::new(), Vec::new(), Vec::new(), vec![ReferenceAnswer::default(); 2]);
        let mut capture = Collector::new(
            &memory,
            0,
            1,
            &mut searchers,
            &mut avoided,
            &mut bytes,
            &mut answers,
        );
        let input = capture.capture(TASK, mission, &prefix);
        capture.finish();
        assert_eq!(input.engagement_target.is_some(), has_target);
        assert_eq!(input.avoided_ship_types, None);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| at == TASK + AVOIDED_SHIP_TYPES)
        );
    }
}

#[test]
fn native_pointer_scratch_is_erased_on_early_drop_and_rust_unwind() {
    let memory = world(1, 17, &[]);
    let mut answers = vec![ReferenceAnswer::default(); 2];
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut reader = Reader::new(&memory, 0, &mut answers);
        assert_eq!(reader.searching_owner(REFERENCE).roster_owner, Some(17));
        panic!("fixture early unwind");
    }));
    assert!(unwind.is_err());
    assert!(
        answers
            .iter()
            .all(|answer| answer.resolution.is_none() && answer.owner.is_none())
    );
    assert_eq!(reference_slots(0), Some(0));
    assert_eq!(reference_slots(3), Some(8));
    assert_eq!(reference_slots(usize::MAX), None);
}

#[test]
fn sizing_preserves_later_valid_lengths_when_an_independent_string_is_bad() {
    let mut memory = world(1, 17, &[REFERENCE, 0]);
    strings(&mut memory.memory, &[b"one", b"bad", b"last"]);
    memory.memory.put(STRINGS + 32 + 16, &99_u64.to_le_bytes());
    let demand = demand(&memory, 0, TASK, 2);
    assert_eq!(
        (
            demand.searchers,
            demand.avoided,
            demand.bytes,
            demand.references
        ),
        (2, 3, 7, 3)
    );
    assert_eq!(
        demand.refusal,
        Some("port sizing avoided string header invalid")
    );
}

#[test]
fn growing_reference_rehash_preserves_the_original_failed_observation() {
    let mut memory = world(1, 17, &[REFERENCE]);
    memory
        .memory
        .put(TASK + ENGAGEMENT_TARGET, &REFERENCE.to_le_bytes());
    let bucket = 0x2000 + Fixture::index(REFERENCE) * engine_native_parts::reference_bucket::STRIDE;
    memory.fail_once.set(Some(bucket));
    let (mut searchers, mut avoided, mut bytes, mut answers) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    for occurrence in 0..193 {
        let input = capture.capture(TASK, 2, &prefix());
        assert_eq!(input.searching, Some((occurrence, 1)));
        assert_eq!(input.engagement_target.unwrap().resolves, None);
    }
    let counts = capture.finish();
    assert_eq!(counts.searching_len, 193);
    assert!(
        searchers[..counts.searching_len].iter().all(|searcher| {
            searcher.reference == Some(REFERENCE) && searcher.resolves.is_none()
        })
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == bucket)
            .count(),
        1
    );
    assert!(answers.iter().all(|answer| answer.resolution.is_none()));
}

#[test]
fn growing_port_buffers_keep_partial_raw_faults_and_later_string_slots() {
    let mut memory = world(1, 17, &[REFERENCE; 193]);
    strings(&mut memory.memory, &[b"one", b"bad", b"tail"]);
    memory.memory.put(STRINGS + 32 + 16, &99_u64.to_le_bytes());
    memory.fail_once.set(Some(REFS + 8));
    let (mut searchers, mut avoided, mut bytes, mut answers) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut capture = Collector::new(
        &memory,
        0,
        1,
        &mut searchers,
        &mut avoided,
        &mut bytes,
        &mut answers,
    );
    let input = capture.capture(TASK, 2, &prefix());
    let counts = capture.finish();
    assert_eq!(input.searching, Some((0, 193)));
    assert_eq!(searchers[0].roster_owner, Some(17));
    assert!(
        searchers[1..193]
            .iter()
            .all(|searcher| *searcher == Searcher::default())
    );
    assert_eq!(&avoided[..3], &[Some((0, 3)), None, Some((3, 4))]);
    assert_eq!(&bytes[..counts.bytes_len], b"onetail");
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == REFS + 8)
            .count(),
        1
    );
    assert!(!memory.reads.borrow().iter().any(|&(at, _)| at == REFS + 16));
}

#[test]
fn growing_port_heap_text_uses_original_descriptor_and_does_not_retry_faults() {
    const PAYLOAD: usize = 0xb000;
    let expected: Vec<u8> = (0..100_003).map(|index| (index % 251) as u8).collect();
    for fail in [false, true] {
        let mut memory = world(1, 17, &[]);
        array(&mut memory.memory, TASK + AVOIDED_SHIP_TYPES, STRINGS, 1);
        let mut header = [0; 32];
        header[..8].copy_from_slice(&(PAYLOAD as u64).to_le_bytes());
        header[16..24].copy_from_slice(&(expected.len() as u64).to_le_bytes());
        header[24..].copy_from_slice(&(expected.len() as u64).to_le_bytes());
        memory.memory.put(STRINGS, &header);
        memory.memory.put(PAYLOAD, &expected);
        memory.memory.put(PAYLOAD + expected.len(), &[0]);
        if fail {
            memory.fail_once.set(Some(PAYLOAD + expected.len() - 1));
        }
        let (mut searchers, mut avoided, mut bytes, mut answers) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut capture = Collector::new(
            &memory,
            0,
            1,
            &mut searchers,
            &mut avoided,
            &mut bytes,
            &mut answers,
        );
        assert_eq!(
            capture.capture(TASK, 2, &prefix()).avoided_ship_types,
            Some((0, 1))
        );
        let counts = capture.finish();
        assert_eq!(avoided[0], (!fail).then_some((0, expected.len())));
        assert_eq!(counts.bytes_len, if fail { 0 } else { expected.len() });
        if !fail {
            assert_eq!(&bytes[..counts.bytes_len], expected);
        }
        let reads = memory.reads.borrow();
        assert_eq!(reads.iter().filter(|&&(at, _)| at == STRINGS).count(), 1);
        let tail = reads
            .iter()
            .position(|&(at, len)| at == PAYLOAD + expected.len() - 1 && len == 2)
            .expect("original last-byte and terminator probe");
        let mut next = PAYLOAD;
        for &(at, len) in &reads[tail + 1..] {
            assert_eq!(
                at, next,
                "each payload byte is acquired once in source order"
            );
            assert!(len > 0 && len <= PAYLOAD + expected.len() - next);
            next += len;
        }
        assert_eq!(
            next,
            if fail {
                PAYLOAD
            } else {
                PAYLOAD + expected.len()
            }
        );
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, _)| at == PAYLOAD + expected.len() - 1)
                .count(),
            1
        );
    }
}
