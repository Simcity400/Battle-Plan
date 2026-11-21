use super::*;
use crate::{
    cattache_relation, cdocking_rights_relation, cgame_state, cguarantee_relation,
    cmarket_access_rights_relation, coffer_military_access_relation, memory::SliceMemory,
};

const BASE: usize = 0;
const GS: usize = 0x1000;
const COUNTRIES: [usize; 3] = [0x4000, 0x5000, 0x6000];
const DIPLOMACY: [usize; 3] = [0x8000, 0x9000, 0xa000];
const RELATION: [usize; 2] = [0x1c000, 0x1c800];

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, values.len() as u32);
    p32(bytes, at + 12, values.len() as u32);
    for (index, value) in values.iter().enumerate() {
        p64(bytes, data + index * 8, *value);
    }
}
fn own(bytes: &mut [u8], owner: usize, relations: &[usize]) {
    let relations: Vec<u64> = relations.iter().map(|&r| r as u64).collect();
    array(
        bytes,
        DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
        0x3400 + owner * 0x40,
        &relations,
    );
}
fn status(owner: usize, target: usize) -> usize {
    0x10000 + owner * 0x4000 + target * 0x1000
}
fn matrix_data(owner: usize) -> usize {
    0x3200 + owner * 0x40
}
fn serializable_data(owner: usize, target: usize) -> usize {
    0x2800 + owner * 0x80 + target * 0x20
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x24000];
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        0x3000,
        &[
            u64::MAX,
            COUNTRIES[0] as u64,
            COUNTRIES[1] as u64,
            COUNTRIES[2] as u64,
        ],
    );
    p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
    for (index, value) in [0_u32, 1, 2, 3].into_iter().enumerate() {
        p32(&mut bytes, 0x3100 + index * 4, value);
    }
    for owner in 0..3 {
        let country = COUNTRIES[owner];
        let diplomacy = DIPLOMACY[owner];
        p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA.0) as u64);
        p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
        p64(
            &mut bytes,
            country + ccountry::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
        p64(
            &mut bytes,
            diplomacy,
            (BASE + cdiplomacy_status::VTABLE_RVA.0) as u64,
        );
        p64(
            &mut bytes,
            diplomacy + cdiplomacy_status::COUNTRY,
            country as u64,
        );
        let slots: Vec<u64> = (0..4).map(|t| status(owner, t) as u64).collect();
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::RELATIONS,
            matrix_data(owner),
            &slots,
        );
        for target in 0..4 {
            let object = status(owner, target);
            p64(&mut bytes, object, (BASE + VTABLE_RVA.0) as u64);
            p64(&mut bytes, object + DIPLOMACY_STATUS, diplomacy as u64);
            p32(&mut bytes, object + TARGET_COUNTRY, target as u32);
            array(
                &mut bytes,
                object + SERIALIZABLE_RELATIONS,
                serializable_data(owner, target),
                &[],
            );
        }
        p32(&mut bytes, status(owner, 0) + TARGET_COUNTRY, 1);
    }
    bytes
}

fn relation(bytes: &mut [u8], object: usize, vtable: Rva, type_id: u32, first: u32, second: u32) {
    p64(bytes, object, (BASE + vtable.0) as u64);
    p32(bytes, object + crelation::TYPE_ID, type_id);
    p32(bytes, object + crelation::FIRST_COUNTRY, first);
    p32(bytes, object + crelation::SECOND_COUNTRY, second);
}

fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

fn country(index: usize) -> ccountry::Country {
    ccountry::Country {
        pointer: COUNTRIES[index],
        id: index as u32 + 1,
        canonical_slot: index as u32 + 1,
    }
}

#[test]
fn zero_relation_keeps_pointer_presence_and_raw_bytes_and_refuses_short_reads() {
    let mut bytes = fixture();
    let zero = status(0, 0);
    p32(&mut bytes, zero + TARGET_COUNTRY, 0);
    bytes[zero + CO_BELLIGERENT] = 1;
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Ok(ZeroRelation {
            co_belligerent: 1,
            ..Default::default()
        })
    );
    p64(&mut bytes, zero + WAR_RELATION, RELATION[0] as u64);
    bytes[RELATION[0] + crelation::RETIRED] = 0x7f;
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Ok(ZeroRelation {
            war_present: 1,
            war_retired: 0x7f,
            co_belligerent: 1
        })
    );
    assert!(
        zero_relation(
            &memory(&bytes[..RELATION[0] + crelation::RETIRED]),
            BASE,
            country(0)
        )
        .is_err()
    );
    p64(&mut bytes, matrix_data(0), 0);
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Err("relation status missing")
    );
}

#[test]
fn co_belligerents_read_each_candidate_byte_in_both_vectors() {
    let mut bytes = fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[3, 0, 2],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3700,
        &[2],
    );
    bytes[status(0, 2) + CO_BELLIGERENT] = 1;
    bytes[status(0, 3) + CO_BELLIGERENT] = 1;
    let memory = memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut slots = vec![0_u32; 4];
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Ok((2, 2))
    );
    assert_eq!(slots[..2], [2, 3]);
    bytes[status(0, 3) + CO_BELLIGERENT] = 0;
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Ok((1, 1))
    );

    p64(&mut bytes, status(0, 2) + DIPLOMACY_STATUS, 0);
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Err("relation status identity mismatch")
    );
}

fn co_inputs_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    links_vector(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        0x3100,
        &[0, 1, 2, 3, 2, 3],
    );
    p32(&mut bytes, status(0, 0) + TARGET_COUNTRY, 0);
    p32(&mut bytes, status(0, 2) + TARGET_COUNTRY, 4);
    p32(&mut bytes, status(0, 3) + TARGET_COUNTRY, 5);
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[3, 0, 4, 2, 4],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3700,
        &[2, 3],
    );
    bytes[status(0, 0) + CO_BELLIGERENT] = 0x80;
    bytes[status(0, 2) + CO_BELLIGERENT] = 0x7f;
    bytes[status(0, 3) + CO_BELLIGERENT] = 1;
    p64(&mut bytes, status(0, 0) + WAR_RELATION, RELATION[0] as u64);
    bytes[RELATION[0] + crelation::RETIRED] = 0x7f;
    bytes
}

const ZERO_COUNTRY: usize = 0x25000;
const ZERO_DIPLOMACY: usize = 0x27000;
const ZERO_MATRIX: usize = 0x31000;
fn zero_status(slot: usize) -> usize {
    0x32000 + slot * OBJECT_SIZE
}

fn reverse_fixture() -> Vec<u8> {
    let mut bytes = co_inputs_fixture();
    bytes.resize(0x38000, 0);
    p64(&mut bytes, 0x3000, ZERO_COUNTRY as u64);
    p64(&mut bytes, ZERO_COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        ZERO_COUNTRY + ccountry::DIPLOMACY_STATUS,
        ZERO_DIPLOMACY as u64,
    );
    p64(
        &mut bytes,
        ZERO_DIPLOMACY,
        cdiplomacy_status::VTABLE_RVA.0 as u64,
    );
    p64(
        &mut bytes,
        ZERO_DIPLOMACY + cdiplomacy_status::COUNTRY,
        ZERO_COUNTRY as u64,
    );
    let slots = std::array::from_fn::<_, 4, _>(|slot| zero_status(slot) as u64);
    array(
        &mut bytes,
        ZERO_DIPLOMACY + cdiplomacy_status::RELATIONS,
        ZERO_MATRIX,
        &slots,
    );
    for slot in 0..4 {
        p64(&mut bytes, zero_status(slot), VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            zero_status(slot) + DIPLOMACY_STATUS,
            ZERO_DIPLOMACY as u64,
        );
        p32(&mut bytes, zero_status(slot) + TARGET_COUNTRY, slot as u32);
    }
    bytes
}

struct ReverseTrace<'a> {
    memory: SliceMemory<'a>,
    fault: Option<usize>,
    attempts: std::cell::Cell<usize>,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
}
impl<'a> ReverseTrace<'a> {
    fn new(bytes: &'a [u8], fault: Option<usize>) -> Self {
        Self {
            memory: memory(bytes),
            fault,
            attempts: std::cell::Cell::new(0),
            calls: Default::default(),
        }
    }
    fn reads(&self, at: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|(address, len)| *address <= at && at - *address < *len)
            .count()
    }
}
impl Memory for ReverseTrace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            let attempts = self.attempts.get();
            self.attempts.set(attempts + 1);
            if attempts == 0 {
                return false;
            }
        }
        self.memory.copy(address, output)
    }
}

fn reverse_source(memory: &impl Memory) -> ReservedVisibilitySource {
    let mut naming = ccountry::read_reserved_naming_observed_inner(memory, BASE, GS, crate::walk::Buffer::Count, None);
    assert!(matches!(naming.outcome, Outcome::Unavailable { .. }));
    let observed = naming.visibility.take().unwrap();
    ReservedVisibilitySource::new(memory, &observed).unwrap()
}

#[test]
fn observed_forward_keeps_the_original_order_values_and_read_faults() {
    let bytes = reverse_fixture();
    for fault in [
        None,
        Some(status(0, 0) + WAR_RELATION),
        Some(RELATION[0] + crelation::RETIRED),
        Some(status(0, 0) + SUBJECT_RELATION),
        Some(status(0, 0) + TARGET_COUNTRY),
        Some(status(0, 2) + CO_BELLIGERENT),
    ] {
        let old = ReverseTrace::new(&bytes, fault);
        let old_countries = ccountry::Reader::new(&old, BASE, GS).unwrap();
        old.calls.borrow_mut().clear();
        let mut old_output = vec![99; 4];
        let expected =
            co_belligerent_inputs(&old, BASE, &old_countries, country(0), &mut old_output);
        let new = ReverseTrace::new(&bytes, fault);
        let new_countries = ccountry::Reader::new(&new, BASE, GS).unwrap();
        new.calls.borrow_mut().clear();
        let mut new_output = vec![99; 4];
        let mut observation = ForwardZeroObservation::default();
        assert_eq!(
            co_belligerent_inputs_observed(
                &new,
                BASE,
                &new_countries,
                country(0),
                &mut new_output,
                &mut observation
            ),
            expected
        );
        assert_eq!(new_output, old_output);
        assert_eq!(*new.calls.borrow(), *old.calls.borrow());
        assert_eq!(new.attempts.get(), old.attempts.get());
        if fault.is_none() {
            assert_eq!(
                &new.calls.borrow()[..12],
                &[
                    (COUNTRIES[0] + ccountry::DIPLOMACY_STATUS, 8),
                    (DIPLOMACY[0], 8),
                    (DIPLOMACY[0] + cdiplomacy_status::COUNTRY, 8),
                    (DIPLOMACY[0] + cdiplomacy_status::RELATIONS, 16),
                    (matrix_data(0), 8),
                    (status(0, 0), 8),
                    (status(0, 0) + DIPLOMACY_STATUS, 8),
                    (status(0, 0) + TARGET_COUNTRY, 4),
                    (status(0, 0) + WAR_RELATION, 8),
                    (RELATION[0] + crelation::RETIRED, 1),
                    (status(0, 0) + SUBJECT_RELATION, 8),
                    (status(0, 0) + CO_BELLIGERENT, 1),
                ]
            );
        }
    }
}

#[test]
fn reverse_zero_cell_and_header_hint_are_independent_of_positive_links_and_names() {
    let mut bytes = reverse_fixture();
    p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0);
    let memory = ReverseTrace::new(&bytes, None);
    assert!(ccountry::Reader::new(&memory, BASE, GS).is_err());
    let mut source = reverse_source(&memory);
    assert_eq!(source.required_slots(), 4);
    assert_eq!(
        source.cell(&memory, BASE, None, 0, None),
        ReverseZeroVisibilityCapture {
            value: Ok(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                subject: Some(ZeroSubjectInputs::default()),
                military_access: Some(0),
            }),
            failures: [None; 4],
        }
    );
    memory.calls.borrow_mut().clear();
    assert_eq!(reserved_visibility_demand(&memory, BASE, GS), Ok(4));
    assert_eq!(memory.reads(ZERO_COUNTRY + ccountry::OWNED_STATE_COUNT), 0);
    assert_eq!(memory.reads(ZERO_DIPLOMACY + cdiplomacy_status::FACTION), 0);
    assert_eq!(memory.reads(crate::ccountry_database::SINGLETON_RVA.0), 0);
    assert_eq!(
        memory.reads(ZERO_DIPLOMACY + cdiplomacy_status::RELATIONS),
        1
    );
    source.clear();
    assert_eq!(source.required_slots(), 4);
    assert_eq!(
        source.cell(&memory, BASE, None, 0, None).value,
        Err("reserved visibility controls cleared")
    );
}

#[test]
fn reverse_cell_authenticates_raw_aliases_including_readable_positive_index_zero() {
    let mut bytes = reverse_fixture();
    links_vector(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        0x3100,
        &[0, 1, 2, 3, 2, 3, 0],
    );
    p32(&mut bytes, zero_status(0) + TARGET_COUNTRY, 6);
    p32(&mut bytes, zero_status(2) + TARGET_COUNTRY, 4);
    let memory = memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let source = reverse_source(&memory);
    assert_eq!(
        source
            .cell(&memory, BASE, Some(&countries), 0, None)
            .value
            .unwrap()
            .target,
        Some(6)
    );
    assert_eq!(
        source
            .cell(&memory, BASE, Some(&countries), 2, None)
            .value
            .unwrap()
            .target,
        Some(4)
    );
    drop(source);
    p32(&mut bytes, zero_status(0) + TARGET_COUNTRY, u32::MAX);
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let source = reverse_source(&memory);
    let refused = source.cell(&memory, BASE, Some(&countries), 0, None);
    assert_eq!(refused.value.unwrap().target, None);
    assert_eq!(
        refused.failures[0],
        Some("country link unreadable or outside country table")
    );
}

#[test]
fn nonnull_reserved_faction_self_match_skips_zero_cell_target_and_subject() {
    let mut bytes = reverse_fixture();
    const FACTION: usize = 0x29000;
    p64(&mut bytes, FACTION, crate::cfaction::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        FACTION + crate::cfaction::REFERENCE,
        0xff00_0000_0000_0058,
    );
    p64(
        &mut bytes,
        ZERO_DIPLOMACY + cdiplomacy_status::FACTION,
        FACTION as u64,
    );
    let memory = ReverseTrace::new(&bytes, Some(zero_status(0) + TARGET_COUNTRY));
    let source = reverse_source(&memory);
    let captured = source.cell(&memory, BASE, None, 0, None);
    assert_eq!(
        captured.value.unwrap(),
        ReverseZeroVisibilityInputs {
            target: None,
            war: Some([0, 0]),
            subject: None,
            military_access: Some(0),
        }
    );
    assert_eq!(captured.failures, [None; 4]);
    assert_eq!(memory.attempts.get(), 0);
    assert_eq!(memory.reads(zero_status(0) + SUBJECT_RELATION), 0);
}

#[test]
fn active_reverse_war_skips_target_and_subject_but_retired_truth_reaches_them() {
    let mut bytes = reverse_fixture();
    p64(
        &mut bytes,
        zero_status(1) + WAR_RELATION,
        RELATION[0] as u64,
    );
    bytes[RELATION[0] + crelation::RETIRED] = 0;
    let memory = ReverseTrace::new(&bytes, Some(zero_status(1) + TARGET_COUNTRY));
    let source = reverse_source(&memory);
    let active = source.cell(&memory, BASE, None, 1, None);
    assert_eq!(
        active.value.unwrap(),
        ReverseZeroVisibilityInputs {
            target: None,
            war: Some([1, 0]),
            subject: None,
            military_access: None,
        }
    );
    assert_eq!(active.failures, [None; 4]);
    assert_eq!(memory.attempts.get(), 0);
    assert_eq!(memory.reads(zero_status(1) + SUBJECT_RELATION), 0);
    drop(source);
    drop(memory);
    bytes[RELATION[0] + crelation::RETIRED] = 0xff;
    let memory = ReverseTrace::new(&bytes, Some(zero_status(1) + TARGET_COUNTRY));
    let source = reverse_source(&memory);
    let retired = source.cell(&memory, BASE, None, 1, None);
    assert_eq!(retired.value.unwrap().war, Some([1, 0xff]));
    assert_eq!(
        retired.failures[0],
        Some("reverse-zero relation target unreadable")
    );
    assert_eq!(memory.attempts.get(), 1);
}

#[test]
fn reverse_reuses_forward_first_zero_and_reads_only_the_previously_skipped_second() {
    let mut bytes = reverse_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        0,
        1,
    );
    bytes[RELATION[1] + crelation::RETIRED] = 0xff;
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    let memory = ReverseTrace::new(&bytes, None);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let source = reverse_source(&memory);
    let mut observed = ForwardZeroObservation::default();
    let forward = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    assert_eq!(forward.zero_subject.unwrap().parties, Some([Some(0), None]));
    let reverse = source.cell(&memory, BASE, Some(&countries), 1, Some(&observed));
    assert_eq!(
        reverse.value.unwrap().subject.unwrap().parties,
        Some([Some(0), Some(1)])
    );
    assert_eq!(reverse.failures, [None; 4]);
    assert_eq!(memory.reads(RELATION[1] + crelation::TYPE_ID), 1);
    assert_eq!(memory.reads(RELATION[1]), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::FIRST_COUNTRY), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::SECOND_COUNTRY), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::RETIRED), 0);
}

#[test]
fn shared_failed_party_is_not_retried_and_known_second_raw_match_survives() {
    let mut bytes = reverse_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        1,
        2,
    );
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(2) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    let memory = ReverseTrace::new(&bytes, Some(RELATION[1] + crelation::FIRST_COUNTRY));
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let source = reverse_source(&memory);
    let mut observed = ForwardZeroObservation::default();
    let forward = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    assert_eq!(
        forward.zero_subject,
        Err("slot-0 subject first party unreadable")
    );
    let reverse = source.cell(&memory, BASE, Some(&countries), 2, Some(&observed));
    assert_eq!(
        reverse.value.unwrap().subject.unwrap().parties,
        Some([None, Some(2)])
    );
    assert_eq!(
        reverse.failures[2],
        Some("slot-0 subject first party unreadable")
    );
    assert_eq!(memory.attempts.get(), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::SECOND_COUNTRY), 1);
}

#[test]
fn reverse_first_match_ignores_forward_second_failure_and_alias_matches_short_circuit() {
    for (slot, target, first) in [(1, 1, 1), (2, 4, 2)] {
        let mut bytes = reverse_fixture();
        p32(&mut bytes, zero_status(slot) + TARGET_COUNTRY, target);
        relation(
            &mut bytes,
            RELATION[1],
            csubject_relation::VTABLE_RVA,
            csubject_relation::TYPE_VALUE,
            first,
            3,
        );
        p64(
            &mut bytes,
            status(0, 0) + SUBJECT_RELATION,
            RELATION[1] as u64,
        );
        p64(
            &mut bytes,
            zero_status(slot) + SUBJECT_RELATION,
            RELATION[1] as u64,
        );
        let memory = ReverseTrace::new(&bytes, Some(RELATION[1] + crelation::SECOND_COUNTRY));
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let source = reverse_source(&memory);
        let mut observed = ForwardZeroObservation::default();
        let forward = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![0; 4],
            &mut observed,
        );
        assert_eq!(
            forward.zero_subject,
            Err("slot-0 subject second party unreadable")
        );
        let reverse = source.cell(
            &memory,
            BASE,
            Some(&countries),
            slot as u32,
            Some(&observed),
        );
        assert_eq!(
            reverse.value.unwrap().subject.unwrap().parties,
            Some([Some(first), None])
        );
        assert_eq!(reverse.failures[2], None);
        assert_eq!(memory.attempts.get(), 1);
    }
}

#[test]
fn forward_null_and_different_pointers_never_certify_reverse_absence_or_retry() {
    let mut bytes = reverse_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        0,
        1,
    );
    p64(
        &mut bytes,
        zero_status(1) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + WAR_RELATION,
        RELATION[1] as u64,
    );
    bytes[RELATION[1] + crelation::RETIRED] = 0x80;
    let memory = ReverseTrace::new(&bytes, Some(RELATION[0] + crelation::RETIRED));
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let source = reverse_source(&memory);
    let mut observed = ForwardZeroObservation::default();
    let forward = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    assert!(forward.zero_war.is_err());
    assert_eq!(forward.zero_subject, Ok(ZeroSubjectInputs::default()));
    let reverse = source.cell(&memory, BASE, Some(&countries), 1, Some(&observed));
    assert_eq!(reverse.value.unwrap().war, Some([1, 0x80]));
    assert_eq!(
        reverse.value.unwrap().subject.unwrap().parties,
        Some([Some(0), Some(1)])
    );
    assert_eq!(reverse.failures, [None; 4]);
    assert_eq!(memory.attempts.get(), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::RETIRED), 1);
}

#[test]
fn same_pointer_retired_type_and_identity_faults_are_sticky_and_independent() {
    let mut bytes = reverse_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        0,
        1,
    );
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + WAR_RELATION,
        RELATION[0] as u64,
    );
    for (at, domain, reason) in [
        (
            RELATION[0] + crelation::RETIRED,
            1,
            "slot-0 war retired byte unreadable",
        ),
        (
            RELATION[1] + crelation::TYPE_ID,
            2,
            "slot-0 subject type unreadable",
        ),
        (RELATION[1], 2, "slot-0 subject vtable mismatch"),
    ] {
        let memory = ReverseTrace::new(&bytes, Some(at));
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let source = reverse_source(&memory);
        let mut observed = ForwardZeroObservation::default();
        co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![0; 4],
            &mut observed,
        );
        let reverse = source.cell(&memory, BASE, Some(&countries), 1, Some(&observed));
        assert_eq!(reverse.failures[domain], Some(reason));
        assert_eq!(memory.attempts.get(), 1);
        let value = reverse.value.unwrap();
        if domain == 1 {
            assert_eq!(value.war, None);
            assert_eq!(value.target, None);
            assert_eq!(value.subject, None);
        } else {
            assert_eq!(value.war, Some([1, 0x7f]));
            assert_eq!(value.target, Some(1));
            assert_eq!(value.subject, None);
        }
    }
}

fn co_inputs(bytes: &[u8], output: &mut Vec<u32>) -> CoBelligerentInputs {
    let memory = memory(bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    co_belligerent_inputs(&memory, BASE, &countries, country(0), output)
}

#[test]
fn co_inputs_keep_directed_native_truth_aliases_and_exact_capacity_demand() {
    let mut bytes = co_inputs_fixture();
    let mut slots = vec![99_u32; 4];
    assert_eq!(
        co_inputs(&bytes, &mut slots),
        CoBelligerentInputs {
            targets: Ok(3),
            required_targets: 3,
            zero_war: Ok([1, 0x7f]),
            zero_subject: Ok(ZeroSubjectInputs::default()),
        }
    );
    assert_eq!(slots, [0, 2, 3, 99]);
    bytes[status(0, 2) + CO_BELLIGERENT] = 0;
    let result = co_inputs(&bytes, &mut slots);
    assert_eq!((result.targets, result.required_targets), (Ok(2), 2));
    assert_eq!(slots[..2], [0, 3]);
    bytes[status(0, 0) + CO_BELLIGERENT] = 0;
    bytes[status(0, 3) + CO_BELLIGERENT] = 0;
    let result = co_inputs(&bytes, &mut slots);
    assert_eq!((result.targets, result.required_targets), (Ok(0), 0));
    bytes[status(0, 2) + CO_BELLIGERENT] = 0x80;
    p32(&mut bytes, status(1, 0) + TARGET_COUNTRY, 0);
    let memory = memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerent_inputs(&memory, BASE, &countries, country(1), &mut slots),
        CoBelligerentInputs {
            targets: Ok(0),
            required_targets: 0,
            zero_war: Ok([0, 0]),
            zero_subject: Ok(ZeroSubjectInputs::default()),
        }
    );
}

#[test]
fn co_inputs_read_slot_zero_even_when_the_candidate_vectors_are_empty() {
    let mut bytes = co_inputs_fixture();
    for (offset, data) in [
        (cdiplomacy_status::WARTIME_ALLIES, 0x3600),
        (cdiplomacy_status::ENEMY_COUNTRIES, 0x3700),
    ] {
        links_vector(&mut bytes, DIPLOMACY[0] + offset, data, &[]);
    }
    p64(&mut bytes, status(0, 0) + WAR_RELATION, 0);
    let mut slots = vec![99_u32; 2];
    assert_eq!(
        co_inputs(&bytes, &mut slots),
        CoBelligerentInputs {
            targets: Ok(1),
            required_targets: 1,
            zero_war: Ok([0, 0]),
            zero_subject: Ok(ZeroSubjectInputs::default()),
        }
    );
    assert_eq!(slots, [0, 99]);
}

struct FailCoReadOnce<'a> {
    memory: SliceMemory<'a>,
    at: usize,
    attempts: std::cell::Cell<usize>,
}

impl Memory for FailCoReadOnce<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address <= self.at && self.at - address < output.len() {
            let attempts = self.attempts.get();
            self.attempts.set(attempts + 1);
            if attempts == 0 {
                return false;
            }
        }
        self.memory.copy(address, output)
    }
}

#[test]
fn co_inputs_refuse_each_operand_once_without_erasing_the_independent_result() {
    let bytes = co_inputs_fixture();
    for (at, targets, required_targets, zero_war) in [
        (
            status(0, 0) + WAR_RELATION,
            Ok(3),
            3,
            Err("slot-0 war pointer unreadable"),
        ),
        (
            RELATION[0] + crelation::RETIRED,
            Ok(3),
            3,
            Err("slot-0 war retired byte unreadable"),
        ),
        (
            status(0, 0) + CO_BELLIGERENT,
            Err("slot-0 co-belligerent byte unreadable"),
            0,
            Ok([1, 0x7f]),
        ),
        (
            DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
            Err("co-belligerent candidate vector unreadable or invalid"),
            1,
            Ok([1, 0x7f]),
        ),
        (
            0x3600,
            Err("diplomacy country vector unreadable"),
            1,
            Ok([1, 0x7f]),
        ),
        (
            status(0, 2) + TARGET_COUNTRY,
            Err("relation status target unreadable"),
            2,
            Ok([1, 0x7f]),
        ),
        (
            status(0, 2) + CO_BELLIGERENT,
            Err("co-belligerent byte unreadable"),
            2,
            Ok([1, 0x7f]),
        ),
        (
            DIPLOMACY[0] + cdiplomacy_status::RELATIONS,
            Err("relation matrix unreadable or invalid"),
            0,
            Err("relation matrix unreadable or invalid"),
        ),
        (
            status(0, 0),
            Err("relation status identity mismatch"),
            0,
            Err("relation status identity mismatch"),
        ),
        (
            status(0, 0) + TARGET_COUNTRY,
            Err("slot-0 relation target mismatch"),
            0,
            Err("slot-0 relation target mismatch"),
        ),
        (
            COUNTRIES[0] + ccountry::DIPLOMACY_STATUS,
            Err("country diplomacy unreadable"),
            0,
            Err("country diplomacy unreadable"),
        ),
    ] {
        let memory = FailCoReadOnce {
            memory: memory(&bytes),
            at,
            attempts: std::cell::Cell::new(0),
        };
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let mut slots = vec![99_u32; 4];
        let zero_subject = if [
            DIPLOMACY[0] + cdiplomacy_status::RELATIONS,
            status(0, 0),
            status(0, 0) + TARGET_COUNTRY,
            COUNTRIES[0] + ccountry::DIPLOMACY_STATUS,
        ]
        .contains(&at)
        {
            zero_war.map(|_| ZeroSubjectInputs::default())
        } else {
            Ok(ZeroSubjectInputs::default())
        };
        assert_eq!(
            co_belligerent_inputs(&memory, BASE, &countries, country(0), &mut slots),
            CoBelligerentInputs {
                targets,
                required_targets,
                zero_war,
                zero_subject,
            },
            "{at:#x}"
        );
        assert_eq!(memory.attempts.get(), 1, "{at:#x}");
        if targets.is_err() {
            assert!(
                slots[..required_targets].iter().all(|&slot| slot == 0),
                "{at:#x}"
            );
            assert!(
                slots[required_targets..].iter().all(|&slot| slot == 99),
                "{at:#x}"
            );
        } else {
            assert_eq!(slots, [0, 2, 3, 99], "{at:#x}");
        }
    }
}

#[test]
fn co_inputs_authenticate_the_target_slot_and_refuse_unsupported_native_width() {
    let mut bytes = co_inputs_fixture();
    let mut slots = vec![99_u32; 4];
    p32(&mut bytes, status(0, 0) + TARGET_COUNTRY, 4);
    assert_eq!(
        co_inputs(&bytes, &mut slots),
        CoBelligerentInputs {
            targets: Err("slot-0 relation target mismatch"),
            required_targets: 0,
            zero_war: Err("slot-0 relation target mismatch"),
            zero_subject: Err("slot-0 relation target mismatch"),
        }
    );
    assert_eq!(slots, [99; 4]);
    p32(&mut bytes, status(0, 0) + TARGET_COUNTRY, 0);
    for raw in [0, 3] {
        p32(&mut bytes, status(0, 2) + TARGET_COUNTRY, raw);
        let result = co_inputs(&bytes, &mut slots);
        assert_eq!(
            result.targets,
            Err("relation status target differs from matrix slot")
        );
        assert_eq!(result.zero_war, Ok([1, 0x7f]));
        assert!(
            slots[..result.required_targets]
                .iter()
                .all(|&slot| slot == 0)
        );
    }
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[1],
    );
    p32(&mut bytes, 0x3100 + 4, 1 << 16);
    p32(&mut bytes, GS + cgame_state::COUNTRIES + 8, (1 << 16) + 1);
    p32(&mut bytes, GS + cgame_state::COUNTRIES + 12, (1 << 16) + 1);
    let result = co_inputs(&bytes, &mut slots);
    assert_eq!(
        result.targets,
        Err("co-belligerent target outside relation matrix")
    );
    assert_eq!(result.zero_war, Ok([1, 0x7f]));
    assert!(
        slots[..result.required_targets]
            .iter()
            .all(|&slot| slot == 0)
    );
}

#[test]
fn co_demand_reads_only_native_headers_and_keeps_other_countries_bounds() {
    let mut bytes = co_inputs_fixture();
    let memory = FailCoReadOnce {
        memory: memory(&bytes),
        at: 0x3600,
        attempts: std::cell::Cell::new(0),
    };
    assert_eq!(co_belligerent_demand(&memory, BASE, country(0)), Ok(8));
    assert_eq!(memory.attempts.get(), 0);
    p32(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES + 12,
        6,
    );
    let memory = self::memory(&bytes);
    assert_eq!(
        co_belligerent_demand(&memory, BASE, country(0)),
        Err("co-belligerent candidate vector unreadable or invalid")
    );
    assert_eq!(co_belligerent_demand(&memory, BASE, country(1)), Ok(1));
    assert_eq!(co_belligerent_demand(&memory, BASE, country(2)), Ok(1));
}

#[test]
fn invasion_co_targets_accept_raw_aliases_without_reading_slot_zero() {
    let mut bytes = co_inputs_fixture();
    p64(&mut bytes, matrix_data(0), 0);
    let memory = memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut slots = vec![99_u32; 4];
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Ok((2, 2))
    );
    assert_eq!(slots[..2], [2, 3]);

    p32(&mut bytes, status(0, 2) + TARGET_COUNTRY, 3);
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Err("relation status target differs from matrix slot")
    );
}

#[test]
fn native_empty_and_zero_only_co_candidates_need_no_positive_headers() {
    for candidates in [&[][..], &[0_u32, 0][..]] {
        let mut bytes = fixture();
        links_vector(
            &mut bytes,
            DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
            0x3600,
            candidates,
        );
        links_vector(
            &mut bytes,
            DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
            0x3700,
            &[],
        );
        p64(&mut bytes, matrix_data(0), 0);
        p32(&mut bytes, GS + cgame_state::COUNTRIES + 12, 0);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 0);
        let memory = memory(&bytes);
        let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
        memo.reset(BASE, GS);
        let countries = memo.view(&memory, BASE, GS);
        assert_eq!(
            co_belligerents_native(&memory, BASE, &countries, country(0), &mut Vec::new()),
            Ok((0, 0))
        );
        assert_eq!(memo.link_required(), 0);
    }
}

#[test]
fn native_alias_zero_is_skipped_but_positive_status_target_remains_binding() {
    let mut bytes = fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[2, 0, 3, 2, 3],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3700,
        &[3],
    );
    p32(&mut bytes, 0x3100 + 2 * 4, 0);
    p64(&mut bytes, matrix_data(0), 0);
    p64(&mut bytes, status(0, 2), 0);
    bytes[status(0, 3) + CO_BELLIGERENT] = 0x7f;
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 4).unwrap();
    memo.reset(BASE, GS);
    let memory = memory(&bytes);
    let countries = memo.view(&memory, BASE, GS);
    let mut slots = vec![0_u32; 1];
    assert_eq!(
        co_belligerents_native(&memory, BASE, &countries, country(0), &mut slots),
        Ok((1, 1))
    );
    assert_eq!(slots, [3]);
    assert_eq!(
        co_belligerents_native_buffer(&memory, BASE, &countries, country(0), crate::walk::Buffer::Count),
        Ok((0, 3))
    );
    for raw in [0, 2] {
        p32(&mut bytes, status(0, 3) + TARGET_COUNTRY, raw);
        let memory = self::memory(&bytes);
        memo.reset(BASE, GS);
        let countries = memo.view(&memory, BASE, GS);
        assert_eq!(
            co_belligerents_native(&memory, BASE, &countries, country(0), &mut slots),
            Err("relation status target differs from matrix slot")
        );
    }
}

type Case<'a> = (&'a str, &'a dyn Fn(&mut Vec<u8>));

fn run(bytes: &[u8]) -> (Walk, Vec<OpinionRow>, Vec<TupleRow>) {
    let (mut resolver, mut baseline) = (Resolver::default(), Baseline::default());
    let passes: Vec<_> = (0..3)
        .map(|_| {
            let mut opinions = vec![OpinionRow::default(); 16];
            let mut tuples = vec![TupleRow::default(); 16];
            let result = walk_with(
                &memory(bytes),
                BASE,
                GS,
                &mut opinions,
                &mut tuples,
                [true; 2],
                &mut resolver,
                &mut baseline,
                Some(Observation {
                    token: (1, 1),
                    marks: Some(&[]),
                }),
                |_| true,
            );
            resolver.grow().unwrap();
            baseline.grow().unwrap();
            (
                result,
                opinions[..result.opinions.len()].to_vec(),
                tuples[..result.tuples.len()].to_vec(),
            )
        })
        .collect();
    assert_eq!(passes[0], passes[1]);
    assert_eq!(passes[0], passes[2]);
    passes[0].clone()
}

#[test]
fn links_come_from_owned_relations_without_walking_the_matrix() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cattache_relation::VTABLE_RVA,
        cattache_relation::TYPE,
        1,
        3,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        2,
    );
    for owner in [0, 2] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64, RELATION[1] as u64],
        );
    }
    p64(&mut bytes, status(2, 1) + ATTACHE_LINK, RELATION[0] as u64);
    p64(&mut bytes, status(1, 3) + AIR_BASE_ACCESS, 1);
    let mut rows = vec![LinkRow::default(); 4];
    let mut resolver = Resolver::default();
    let outcome = link_rows(&bytes, &mut rows, &mut resolver);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0],
        LinkRow {
            country: 3,
            target: 1,
            links: LINK_ATTACHE,
            transport_subject: None,
        }
    );
    assert_eq!(
        link_count(&bytes, &mut resolver),
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    p64(&mut bytes, status(2, 1), 7);
    assert!(matches!(
        link_rows(&bytes, &mut rows, &mut resolver),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn a_pair_named_twice_is_one_row_and_the_walk_reuses_its_scratch() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cattache_relation::VTABLE_RVA,
        cattache_relation::TYPE,
        3,
        1,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cair_base_access_relation::VTABLE_RVA,
        cair_base_access_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS,
        0x3400,
        &[RELATION[0] as u64, RELATION[1] as u64],
    );
    p64(&mut bytes, status(0, 3) + ATTACHE_LINK, RELATION[0] as u64);
    p64(
        &mut bytes,
        status(0, 3) + AIR_BASE_ACCESS,
        RELATION[1] as u64,
    );
    let mut resolver = Resolver::with_capacity(8);
    let storage = (resolver.references.as_ptr(), resolver.capacity());
    let mut rows = vec![LinkRow::default(); 4];
    for _ in 0..2 {
        assert_eq!(
            link_rows(&bytes, &mut rows, &mut resolver),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows[0],
            LinkRow {
                country: 1,
                target: 3,
                links: LINK_ATTACHE | LINK_AIR_BASE_ACCESS,
                transport_subject: None,
            }
        );
        assert_eq!((resolver.references.as_ptr(), resolver.capacity()), storage);
    }
    assert_eq!(
        link_count(&bytes, &mut resolver),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
}

fn link_rows(bytes: &[u8], rows: &mut Vec<LinkRow>, resolver: &mut Resolver) -> Outcome {
    link_rows_into(bytes, crate::walk::Buffer::Growing(rows), resolver)
}

fn link_count(bytes: &[u8], resolver: &mut Resolver) -> Outcome {
    link_rows_into(bytes, crate::walk::Buffer::Count, resolver)
}

fn link_rows_into(
    bytes: &[u8],
    rows: crate::walk::Buffer<'_, LinkRow>,
    resolver: &mut Resolver,
) -> Outcome {
    let (mut wars, mut countries) = (Vec::new(), Vec::new());
    links_observed_inner(&memory(bytes), BASE, GS, rows, WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        }.into(), resolver, None, None)
    .links
}

fn transport_links(memory: &impl Memory, rows: &mut Vec<LinkRow>) -> LinkWalk {
    let (mut wars, mut countries) = (Vec::new(), Vec::new());
    links_observed_transport(
        memory,
        BASE,
        GS,
        rows,
        WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        }
        .into(),
        &mut Resolver::default(),
        73,
    )
}

fn transport_subject_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        1,
        2,
    );
    bytes[RELATION[0] + crelation::RETIRED] = 255;
    p32(&mut bytes, RELATION[0] + crelation::START_HOUR, 500);
    p32(&mut bytes, RELATION[0] + crelation::END_HOUR, 501);
    for owner in 0..2 {
        own(&mut bytes, owner, &[RELATION[0], RELATION[0]]);
        p64(
            &mut bytes,
            status(owner, 2 - owner) + SUBJECT_RELATION,
            RELATION[0] as u64,
        );
    }
    bytes
}

#[test]
fn transport_subject_original_sparse_pass_certifies_owners_and_deduplicates_attempts() {
    let bytes = transport_subject_fixture();
    let trace = ReverseTrace::new(&bytes, None);
    let mut rows = vec![LinkRow::default(); 8];
    assert_eq!(
        transport_links(&trace, &mut rows).links,
        Outcome::Rows { len: 5 }
    );
    for (at, owner) in [(0, 1), (2, 2), (4, 3)] {
        assert_eq!(
            (rows[at].country, rows[at].target, rows[at].links),
            (owner, owner, 0)
        );
        assert_eq!(
            rows[at].transport_subject,
            Some(Ok(TransportSubject {
                sample_hour: 73,
                complete_owner: true,
                ..Default::default()
            }))
        );
    }
    assert_eq!(
        rows[1]
            .transport_subject
            .unwrap()
            .unwrap()
            .subject
            .unwrap()
            .parties,
        Some([Some(1), Some(2)])
    );
    assert_eq!(
        rows[3]
            .transport_subject
            .unwrap()
            .unwrap()
            .subject
            .unwrap()
            .parties,
        Some([Some(1), None])
    );
    for owner in 0..2 {
        assert_eq!(trace.reads(status(owner, 2 - owner) + SUBJECT_RELATION), 1);
    }
    assert_eq!(trace.reads(RELATION[0] + crelation::START_HOUR), 0);
    assert_eq!(trace.reads(RELATION[0] + crelation::END_HOUR), 0);
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn transport_subject_only_fault_refuses_coverage_without_erasing_original_tables() {
    let mut bytes = transport_subject_fixture();
    p64(&mut bytes, status(0, 2), 7);
    let mut rows = vec![LinkRow::default(); 8];
    let walk = transport_links(&memory(&bytes), &mut rows);
    assert_eq!(walk.links, Outcome::Rows { len: 4 });
    assert_eq!(walk.wars, Outcome::Rows { len: 3 });
    assert_eq!(
        (
            walk.countries_len,
            walk.required_countries,
            walk.ordered_refusals
        ),
        (0, 0, 1)
    );
    assert_eq!(
        rows[0].transport_subject,
        Some(Err("transport subject status identity unsupported"))
    );
    assert!(rows[1].transport_subject.unwrap().unwrap().complete_owner);
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn transport_subject_merge_keeps_original_link_faults_and_failed_suffix_is_not_retried() {
    let mut bytes = transport_subject_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        cmilitary_access_relation::VTABLE_RVA,
        cmilitary_access_relation::TYPE,
        1,
        2,
    );
    own(&mut bytes, 0, &[RELATION[0], RELATION[1], RELATION[0]]);
    p64(
        &mut bytes,
        status(0, 2) + MILITARY_ACCESS,
        RELATION[1] as u64,
    );
    let mut rows = vec![LinkRow::default(); 8];
    let persistent = StarlessFault {
        memory: memory(&bytes),
        at: status(0, 2) + SUBJECT_RELATION,
    };
    assert_eq!(
        transport_links(&persistent, &mut rows).links,
        Outcome::Rows { len: 5 }
    );
    assert_eq!(rows[1].links, LINK_MILITARY_ACCESS);
    assert_eq!(
        rows[1].transport_subject,
        Some(Err("transport subject pointer unreadable"))
    );
    let trace = ReverseTrace::new(&bytes, Some(status(0, 2) + SUBJECT_RELATION));
    assert_eq!(
        transport_links(&trace, &mut rows).links,
        Outcome::Rows { len: 5 }
    );
    assert_eq!(rows[1].links, LINK_MILITARY_ACCESS);
    assert!(rows[1].transport_subject.unwrap().is_ok());
    assert_eq!(trace.reads(status(0, 2) + SUBJECT_RELATION), 2);
    let trace = ReverseTrace::new(&bytes, Some(status(0, 2) + AIR_BASE_ACCESS));
    assert_eq!(
        transport_links(&trace, &mut rows).links,
        Outcome::Rows { len: 5 }
    );
    let persistent = StarlessFault {
        memory: memory(&bytes),
        at: status(0, 2) + AIR_BASE_ACCESS,
    };
    assert_eq!(
        transport_links(&persistent, &mut rows).links,
        Outcome::Unavailable {
            reason: "relation status link fields unreadable"
        }
    );
}

struct StarlessFault<'a> {
    memory: SliceMemory<'a>,
    at: usize,
}
impl Memory for StarlessFault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        !(address <= self.at && self.at - address < output.len())
            && self.memory.copy(address, output)
    }
}

fn links_vector(bytes: &mut [u8], at: usize, data: usize, values: &[u32]) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, values.len() as u32);
    p32(bytes, at + 12, values.len() as u32);
    for (index, value) in values.iter().enumerate() {
        p32(bytes, data + index * 4, *value);
    }
}

fn war_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cmilitary_access_relation::VTABLE_RVA,
        cmilitary_access_relation::TYPE,
        1,
        2,
    );
    for owner in [0, 1] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64],
        );
    }
    p64(
        &mut bytes,
        status(0, 2) + MILITARY_ACCESS,
        RELATION[0] as u64,
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3600,
        &[3],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3640,
        &[2, 2],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[2] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3680,
        &[1, 2],
    );
    bytes[COUNTRIES[0] + ccountry::CAN_OCCUPY_NON_WAR] = 1;
    bytes[COUNTRIES[2] + ccountry::CAN_OCCUPY_NON_WAR] = 0x80;
    bytes
}

fn walk_wars(bytes: &[u8]) -> (LinkWalk, Vec<WarRow>, Vec<u32>) {
    let mut rows = Vec::new();
    let mut wars = Vec::new();
    let mut countries = Vec::new();
    let walk = links_observed_inner(&memory(bytes), BASE, GS, crate::walk::Buffer::Growing(&mut rows), WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        }.into(), &mut Resolver::default(), None, None);
    (walk, wars, countries)
}

fn row(country: u32, rule: u8) -> WarRow {
    WarRow {
        country,
        can_occupy_non_war: rule,
        ..WarRow::default()
    }
}

#[test]
fn war_rows_record_the_rule_and_leave_enemy_sets_to_derivation() {
    let bytes = war_fixture();
    let (walk, wars, _) = walk_wars(&bytes);
    assert_eq!(walk.links, Outcome::Rows { len: 1 });
    assert_eq!(walk.wars, Outcome::Rows { len: 2 });
    assert_eq!((walk.countries_len, walk.required_countries), (0, 0));
    assert_eq!(wars[..2], [row(1, 1), row(3, 0x80)]);
    let mut rows = vec![LinkRow::default(); 2];
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].links, LINK_MILITARY_ACCESS);
}

#[test]
fn a_civil_war_revolter_has_its_enemy_vector_read() {
    let mut bytes = war_fixture();
    p32(&mut bytes, COUNTRIES[1] + ccountry::CIVIL_WAR_LINK, 1);
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x36c0,
        &[3, 9],
    );
    let (walk, wars, countries) = walk_wars(&bytes);
    assert_eq!(walk.wars, Outcome::Rows { len: 3 });
    assert_eq!(
        wars[1],
        WarRow {
            country: 2,
            can_occupy_non_war: 0,
            enemies_read: true,
            enemies_start: 0,
            enemies_len: 2,
            ordered: None,
            ordered_hour: None,
            ordered_failure: None,
        }
    );
    assert_eq!(countries[..2], [3, 9]);
}

#[test]
fn a_retired_war_relation_makes_its_owner_read_the_vector() {
    let mut bytes = war_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        cwar_relation::VTABLE_RVA,
        cwar_relation::TYPE,
        3,
        1,
    );
    bytes[RELATION[1] + crelation::RETIRED] = 1;
    array(
        &mut bytes,
        DIPLOMACY[2] + cdiplomacy_status::OWNED_RELATIONS,
        0x3480,
        &[RELATION[1] as u64],
    );
    let (walk, wars, countries) = walk_wars(&bytes);
    assert_eq!(walk.wars, Outcome::Rows { len: 2 });
    assert_eq!(wars[0], row(1, 1));
    assert_eq!(
        wars[1],
        WarRow {
            enemies_read: true,
            enemies_len: 2,
            ..row(3, 0x80)
        }
    );
    assert_eq!(countries[..2], [1, 2]);
    bytes[RELATION[1] + crelation::RETIRED] = 0;
    let (walk, wars, _) = walk_wars(&bytes);
    assert_eq!(walk.countries_len, 0);
    assert_eq!(wars[1], row(3, 0x80));
}

#[test]
fn access_and_docking_slots_come_from_their_owned_relations() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cdocking_rights_relation::VTABLE_RVA,
        cdocking_rights_relation::TYPE,
        1,
        3,
    );
    relation(
        &mut bytes,
        RELATION[1],
        coffer_military_access_relation::VTABLE_RVA,
        coffer_military_access_relation::TYPE,
        1,
        3,
    );
    for owner in [0, 2] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64, RELATION[1] as u64],
        );
    }
    p64(
        &mut bytes,
        status(0, 3) + DOCKING_RIGHTS,
        RELATION[0] as u64,
    );
    p64(
        &mut bytes,
        status(2, 1) + MILITARY_ACCESS,
        RELATION[1] as u64,
    );
    let mut rows = vec![LinkRow::default(); 4];
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rows[..2],
        [
            LinkRow {
                country: 1,
                target: 3,
                links: LINK_DOCKING_RIGHTS,
                transport_subject: None,
            },
            LinkRow {
                country: 3,
                target: 1,
                links: LINK_MILITARY_ACCESS,
                transport_subject: None,
            },
        ]
    );
}

#[test]
fn only_nonzero_opinions_and_serializable_relations_become_rows() {
    let mut bytes = fixture();
    p32(&mut bytes, status(0, 2) + OPINION, (-100_i32) as u32);
    p32(&mut bytes, status(1, 3) + OPINION, 60);
    p32(&mut bytes, status(2, 0) + OPINION, 7);
    relation(
        &mut bytes,
        RELATION[0],
        cmarket_access_rights_relation::VTABLE_RVA,
        cmarket_access_rights_relation::TYPE,
        1,
        3,
    );
    p32(&mut bytes, RELATION[0] + crelation::START_HOUR, 100);
    p32(
        &mut bytes,
        RELATION[0] + crelation::END_HOUR,
        (-5_i32) as u32,
    );
    bytes[RELATION[0] + crelation::CANCEL] = 0x80;
    bytes[RELATION[0] + crelation::RETIRED] = 0x02;
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64, RELATION[1] as u64],
    );
    for owner in [0, 2] {
        own(&mut bytes, owner, &RELATION);
    }
    let (result, opinions, tuples) = run(&bytes);
    assert_eq!(
        dense::walk(
            &memory(&bytes),
            BASE,
            GS,
            &mut vec![OpinionRow::default(); 16],
            &mut vec![TupleRow::default(); 16],
            &mut Resolver::default(),
        ),
        result
    );
    assert_eq!(result.opinions, Outcome::Rows { len: 2 });
    assert_eq!(result.tuples, Outcome::Rows { len: 2 });
    assert_eq!(
        opinions,
        vec![
            OpinionRow {
                owner_country: 1,
                target_country: 2,
                opinion: -100,
            },
            OpinionRow {
                owner_country: 2,
                target_country: 3,
                opinion: 60,
            },
        ]
    );
    assert_eq!(
        tuples[0],
        TupleRow {
            owner_country: 1,
            target_country: 3,
            type_id: cmarket_access_rights_relation::TYPE,
            ordinal: 0,
            first_country: 1,
            second_country: 3,
            start_hour: 100,
            end_hour: -5,
            cancel: 0x80,
            retired: 0x02,
        }
    );
    assert_eq!(tuples[1].ordinal, 1);
    assert_eq!(tuples[1].type_id, cguarantee_relation::TYPE);
}

#[test]
fn war_and_subject_elements_are_skipped_but_still_consume_their_ordinal() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cwar_relation::VTABLE_RVA,
        cwar_relation::TYPE,
        1,
        3,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    let subject = 0x1d000;
    let market = 0x1d800;
    relation(
        &mut bytes,
        subject,
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        1,
        3,
    );
    relation(
        &mut bytes,
        market,
        cmarket_access_rights_relation::VTABLE_RVA,
        cmarket_access_rights_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[
            RELATION[0] as u64,
            RELATION[1] as u64,
            subject as u64,
            market as u64,
        ],
    );
    for owner in [0, 2] {
        own(
            &mut bytes,
            owner,
            &[RELATION[0], RELATION[1], subject, market],
        );
    }
    let (result, _, tuples) = run(&bytes);
    assert_eq!(result.tuples, Outcome::Rows { len: 2 });
    assert_eq!(
        tuples
            .iter()
            .map(|row| (row.type_id, row.ordinal))
            .collect::<Vec<_>>(),
        vec![
            (cguarantee_relation::TYPE, 1),
            (cmarket_access_rights_relation::TYPE, 3),
        ]
    );
    let mut broken = bytes.clone();
    p32(&mut broken, RELATION[0] + crelation::TYPE_ID, 1);
    assert_eq!(
        run(&broken).0.tuples,
        Outcome::Unavailable {
            reason: "relation type word differs from its class"
        }
    );
}

#[test]
fn empty_matrices_are_a_complete_empty_pair_of_tables() {
    let (result, opinions, tuples) = run(&fixture());
    assert_eq!(result.opinions, Outcome::Rows { len: 0 });
    assert_eq!(result.tuples, Outcome::Rows { len: 0 });
    assert!(opinions.is_empty() && tuples.is_empty());
}

#[test]
fn changed_headers_null_slots_and_bad_identities_are_named_failures() {
    let mut valid = fixture();
    relation(
        &mut valid,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut valid,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64],
    );
    for owner in [0, 2] {
        own(&mut valid, owner, &[RELATION[0]]);
    }
    assert_eq!(run(&valid).0.tuples, Outcome::Rows { len: 1 });
    let cases: [Case<'_>; 12] = [
        ("relation status vtable mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, status(0, 2), 0);
        }),
        ("null relation matrix entry", &|bytes: &mut Vec<u8>| {
            p64(bytes, matrix_data(0) + 2 * 8, 0);
        }),
        (
            "relation status target differs from matrix slot",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, status(0, 2) + TARGET_COUNTRY, 3);
            },
        ),
        (
            "relation matrix does not cover country ordinals",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, DIPLOMACY[0] + cdiplomacy_status::RELATIONS + 12, 2);
            },
        ),
        ("diplomacy vtable mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, DIPLOMACY[1], 0);
        }),
        ("diplomacy country mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, DIPLOMACY[1] + cdiplomacy_status::COUNTRY, 0x1234);
        }),
        ("country diplomacy is null", &|bytes: &mut Vec<u8>| {
            p64(bytes, COUNTRIES[1] + ccountry::DIPLOMACY_STATUS, 0);
        }),
        (
            "serializable relation vector invalid",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, status(0, 3) + SERIALIZABLE_RELATIONS + 12, 9);
            },
        ),
        (
            "owned relation vector unreadable or invalid",
            &|bytes: &mut Vec<u8>| {
                p32(
                    bytes,
                    DIPLOMACY[2] + cdiplomacy_status::OWNED_RELATIONS + 12,
                    9,
                );
            },
        ),
        (
            "owned relation pointer null or invalid",
            &|bytes: &mut Vec<u8>| {
                p64(bytes, 0x3400 + 2 * 0x40, 0);
            },
        ),
        (
            "owned and serializable relations disagree",
            &|bytes: &mut Vec<u8>| {
                array(bytes, status(0, 3) + SERIALIZABLE_RELATIONS, 0, &[]);
            },
        ),
        (
            "owned and serializable relations disagree",
            &|bytes: &mut Vec<u8>| {
                array(
                    bytes,
                    status(0, 3) + SERIALIZABLE_RELATIONS,
                    serializable_data(0, 3),
                    &[RELATION[0] as u64, RELATION[0] as u64],
                );
            },
        ),
    ];
    let (complete, complete_opinions, _) = run(&valid);
    for (reason, break_it) in cases {
        let mut bytes = valid.clone();
        break_it(&mut bytes);
        let (result, opinions, _) = run(&bytes);
        if reason.starts_with("owned ") {
            assert_eq!(
                (result.opinions, opinions),
                (complete.opinions, complete_opinions.clone()),
                "{reason}"
            );
        } else {
            assert_eq!(result.opinions, Outcome::Unavailable { reason }, "{reason}");
        }
        assert_eq!(result.tuples, Outcome::Unavailable { reason });
    }
    let mut bytes = valid.clone();
    p64(&mut bytes, 0x3400 + 2 * 0x40, 0);
    let mut opinions = vec![OpinionRow::default(); 4];
    let result = walk(
        &memory(&bytes),
        BASE,
        GS,
        &mut opinions,
        &mut Vec::new(),
        [true, false],
    );
    assert_eq!(result.opinions, Outcome::Rows { len: 0 });
}

#[test]
fn unclassified_and_contradictory_relation_identities_are_refused() {
    let mut valid = fixture();
    relation(
        &mut valid,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut valid,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64],
    );
    for owner in [0, 2] {
        own(&mut valid, owner, &[RELATION[0]]);
    }
    assert_eq!(run(&valid).0.tuples, Outcome::Rows { len: 1 });
    let cases: [Case<'_>; 4] = [
        (
            "serializable relation has an unclassified vtable",
            &|bytes: &mut Vec<u8>| p64(bytes, RELATION[0], 0x1234),
        ),
        (
            "relation type word differs from its class",
            &|bytes: &mut Vec<u8>| p32(bytes, RELATION[0] + crelation::TYPE_ID, 1),
        ),
        (
            "serializable relation is not owned by its first endpoint",
            &|bytes: &mut Vec<u8>| {
                relation(
                    bytes,
                    RELATION[1],
                    cguarantee_relation::VTABLE_RVA,
                    cguarantee_relation::TYPE,
                    2,
                    3,
                );
                array(
                    bytes,
                    status(0, 3) + SERIALIZABLE_RELATIONS,
                    serializable_data(0, 3),
                    &[RELATION[0] as u64, RELATION[1] as u64],
                );
            },
        ),
        ("null serializable relation entry", &|bytes: &mut Vec<
            u8,
        >| {
            p64(bytes, serializable_data(0, 3), 0)
        }),
    ];
    for (reason, break_it) in cases {
        let mut bytes = valid.clone();
        break_it(&mut bytes);
        assert_eq!(
            run(&bytes).0.tuples,
            Outcome::Unavailable { reason },
            "{reason}"
        );
    }
}

#[test]
fn a_truncated_address_space_never_yields_a_partial_success() {
    let bytes = fixture();
    for cut in [status(2, 3) + OPINION, matrix_data(2) + 8] {
        let (result, _, _) = run(&bytes[..cut]);
        assert!(
            matches!(result.opinions, Outcome::Unavailable { .. }),
            "{cut:#x} {result:?}"
        );
        assert!(matches!(result.tuples, Outcome::Unavailable { .. }));
    }
}

fn observed(
    bytes: &[u8],
    baseline: &mut Baseline,
    token: (u64, u64),
    marks: &[Mark],
) -> (Outcome, Vec<OpinionRow>, usize) {
    observed_slice(bytes, baseline, token, marks, &|_| true)
}

fn observed_slice(
    bytes: &[u8],
    baseline: &mut Baseline,
    token: (u64, u64),
    marks: &[Mark],
    select: &dyn Fn(u32) -> bool,
) -> (Outcome, Vec<OpinionRow>, usize) {
    let slice = memory(bytes);
    let memory = Counting::new(&slice);
    let mut opinions = vec![OpinionRow::default(); 8];
    let result = walk_with(
        &memory,
        BASE,
        GS,
        &mut opinions,
        &mut Vec::new(),
        [true, false],
        &mut Resolver::default(),
        baseline,
        Some(Observation {
            token,
            marks: Some(marks),
        }),
        select,
    );
    baseline.grow().unwrap();
    (
        result.opinions,
        opinions[..result.opinions.len()].to_vec(),
        memory.take()[1],
    )
}

#[test]
fn observed_walks_replay_the_baseline_and_read_only_marked_statuses() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    let row = |owner_country, target_country, opinion| OpinionRow {
        owner_country,
        target_country,
        opinion,
    };
    let mark = |owner: usize, target: usize| Mark {
        status: status(owner, target),
        diplomacy: DIPLOMACY[owner],
    };
    let cold = observed(&bytes, &mut baseline, token, &[]);
    let sized = observed(&bytes, &mut baseline, token, &[]);
    let recording = observed(&bytes, &mut baseline, token, &[]);
    let reused = baseline.counts().reused_owners;
    let replay = observed(&bytes, &mut baseline, token, &[]);
    assert_eq!((cold.0, &cold.1), (sized.0, &sized.1));
    assert_eq!((cold.0, &cold.1), (recording.0, &recording.1));
    assert_eq!((replay.0, &replay.1), (cold.0, &cold.1));
    assert_eq!(cold.1, [row(2, 2, 5)]);
    assert_eq!(baseline.valid_owners(), 3);
    assert!(replay.2 + 9 * (16 + 8) <= cold.2, "{replay:?} {cold:?}");
    assert!(replay.2 < recording.2, "{replay:?} {recording:?}");
    assert_eq!(baseline.counts().reused_owners - reused, 3);
    let mut unobserved = bytes.clone();
    p32(&mut unobserved, status(0, 3) + OPINION, 9);
    p64(&mut unobserved, status(2, 1), 0);
    assert_eq!(observed(&unobserved, &mut baseline, token, &[]).1, cold.1);
    baseline.reserve_marks(3).unwrap();
    let mut written = bytes.clone();
    p32(&mut written, status(0, 3) + OPINION, 9);
    p32(&mut written, status(2, 1) + OPINION, (-4_i32) as u32);
    let marks = [mark(2, 1), mark(0, 3), mark(2, 1)];
    let (outcome, rows, _) = observed(&written, &mut baseline, token, &marks);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(rows, [row(1, 3, 9), row(2, 2, 5), row(3, 1, -4)]);
    p32(&mut written, status(1, 2) + OPINION, 0);
    let (_, rows, _) = observed(&written, &mut baseline, token, &[mark(1, 2)]);
    assert_eq!(rows, [row(1, 3, 9), row(3, 1, -4)]);
    assert_eq!(baseline.counts().marks, 4);
    let mut moved = written.clone();
    let copy = 0x3300;
    let entries: Vec<u64> = (0..4).map(|t| status(1, t) as u64).collect();
    array(
        &mut moved,
        DIPLOMACY[1] + cdiplomacy_status::RELATIONS,
        copy,
        &entries,
    );
    p32(&mut moved, status(1, 1) + OPINION, 6);
    let (_, rows, _) = observed(&moved, &mut baseline, token, &[]);
    assert_eq!(rows, [row(1, 3, 9), row(2, 1, 6), row(3, 1, -4)]);
    let mut stray = moved.clone();
    let fake = 0x20000;
    stray[fake..fake + 0x400].copy_from_slice(&moved[status(0, 2)..status(0, 2) + 0x400]);
    p64(&mut stray, status(2, 3), 0);
    let refused = observed(
        &stray,
        &mut baseline,
        token,
        &[Mark {
            status: fake,
            diplomacy: DIPLOMACY[0],
        }],
    );
    assert_eq!(
        refused.0,
        Outcome::Unavailable {
            reason: "relation status vtable mismatch"
        }
    );
    assert_eq!(baseline.counts().refused_observations, 1);
    p32(&mut moved, status(2, 3) + OPINION, 2);
    let (_, rows, read) = observed(&moved, &mut baseline, token, &[]);
    assert_eq!(
        rows,
        [row(1, 3, 9), row(2, 1, 6), row(3, 1, -4), row(3, 3, 2)]
    );
    assert_eq!(read, cold.2);
    p32(&mut moved, status(0, 1) + OPINION, 1);
    assert_eq!(observed(&moved, &mut baseline, token, &[]).1.len(), 4);
    assert_eq!(observed(&moved, &mut baseline, (1, 2), &[]).1.len(), 5);
    struct Panicking<'a>(SliceMemory<'a>);
    impl Memory for Panicking<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let third = DIPLOMACY[2]..DIPLOMACY[2] + 0x100;
            assert!(!third.contains(&address), "fault injected mid-walk");
            self.0.copy(address, output)
        }
    }
    observed(&moved, &mut baseline, (1, 2), &[]);
    assert!(baseline.token().is_some());
    let fresh = baseline.counts().full_walks;
    let replays = baseline.counts().incremental_walks;
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        walk_with(
            &Panicking(memory(&moved)),
            BASE,
            GS,
            &mut vec![OpinionRow::default(); 8],
            &mut Vec::new(),
            [true, false],
            &mut Resolver::default(),
            &mut baseline,
            Some(Observation {
                token: (1, 2),
                marks: Some(&[]),
            }),
            |_| true,
        )
    }));
    assert!(panicked.is_err());
    assert_eq!(baseline.counts().incremental_walks, replays + 1);
    assert_eq!(baseline.token(), None);
    p32(&mut moved, status(2, 2) + OPINION, 8);
    let (_, rows, _) = observed(&moved, &mut baseline, (1, 2), &[]);
    assert!(rows.contains(&row(3, 2, 8)), "{rows:?}");
    let _ = fresh;
    let mut tuples = vec![TupleRow::default(); 4];
    walk_with(
        &memory(&moved),
        BASE,
        GS,
        &mut Vec::new(),
        &mut tuples,
        [false, true],
        &mut Resolver::default(),
        &mut baseline,
        None,
        |_| true,
    );
    let incremental = baseline.counts().incremental_walks;
    observed(&moved, &mut baseline, (1, 2), &[]);
    assert_eq!(baseline.counts().incremental_walks, incremental + 1);
    let mut short = vec![OpinionRow::default(); 2];
    let result = walk_with(
        &memory(&moved),
        BASE,
        GS,
        &mut short,
        &mut Vec::new(),
        [true, false],
        &mut Resolver::default(),
        &mut baseline,
        Some(Observation {
            token: (1, 2),
            marks: Some(&[]),
        }),
        |_| true,
    );
    assert_eq!(result.opinions, Outcome::Rows { len: 6 });
    assert_eq!(baseline.counts().incremental_walks, incremental + 2);
}

#[test]
fn sliced_observed_walks_keep_other_owners_marks_for_their_slice() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    let mark = |owner: usize, target: usize| Mark {
        status: status(owner, target),
        diplomacy: DIPLOMACY[owner],
    };
    for _ in 0..3 {
        observed(&bytes, &mut baseline, token, &[]);
    }
    assert_eq!(baseline.valid_owners(), 3);
    baseline.reserve_marks(2).unwrap();
    let only = |id: u32| move |owner: u32| owner == id;
    let unobserved = |bytes: &[u8], id: u32| {
        observed_slice(bytes, &mut Baseline::default(), token, &[], &only(id)).1
    };
    p32(&mut bytes, status(0, 3) + OPINION, 9);
    p32(&mut bytes, status(2, 1) + OPINION, (-4_i32) as u32);
    let (_, rows, _) = observed_slice(
        &bytes,
        &mut baseline,
        token,
        &[mark(0, 3), mark(2, 1)],
        &only(1),
    );
    assert_eq!(rows, unobserved(&bytes, 1));
    assert_eq!(baseline.counts().marks, 1);
    let (_, rows, _) = observed_slice(&bytes, &mut baseline, token, &[], &only(3));
    assert_eq!(rows, unobserved(&bytes, 3));
    assert!(rows.iter().any(|row| row.opinion == -4), "{rows:?}");
    assert_eq!(baseline.counts().marks, 2);
    let invalidated = baseline.counts().invalidated_owners;
    let flood = vec![mark(1, 3); 128];
    observed_slice(&bytes, &mut baseline, token, &flood, &only(1));
    assert_eq!(baseline.counts().invalidated_owners, invalidated + 1);
    let before = baseline.counts().read_owners;
    p32(&mut bytes, status(1, 1) + OPINION, 7);
    let (_, rows, _) = observed_slice(&bytes, &mut baseline, token, &[], &only(2));
    assert_eq!(rows, unobserved(&bytes, 2));
    assert_eq!(baseline.counts().read_owners, before + 1);
}

#[test]
fn marks_beyond_either_buffer_read_their_owners_in_full() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    for _ in 0..3 {
        observed(&bytes, &mut baseline, token, &[]);
    }
    assert_eq!(baseline.valid_owners(), 3);
    baseline.shorten_updates(0);
    p32(&mut bytes, status(0, 3) + OPINION, 9);
    let mark = Mark {
        status: status(0, 3),
        diplomacy: DIPLOMACY[0],
    };
    let invalidated = baseline.counts().invalidated_owners;
    let (outcome, rows, _) = observed(&bytes, &mut baseline, token, &[mark]);
    let (reference_outcome, reference, _) = observed(&bytes, &mut Baseline::default(), token, &[]);
    assert_eq!((outcome, rows), (reference_outcome, reference));
    assert_eq!(baseline.counts().invalidated_owners, invalidated + 3);
}

#[test]
fn received_relations_and_null_endpoints_name_no_status() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        2,
        1,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        0,
    );
    array(
        &mut bytes,
        status(1, 1) + SERIALIZABLE_RELATIONS,
        serializable_data(1, 1),
        &[RELATION[0] as u64],
    );
    own(&mut bytes, 0, &RELATION);
    own(&mut bytes, 1, &[RELATION[0]]);
    let slice = memory(&bytes);
    let counting = Counting::new(&slice);
    let mut tuples = vec![TupleRow::default(); 4];
    let result = walk(&counting, BASE, GS, &mut Vec::new(), &mut tuples, [false, true]);
    assert_eq!(result.tuples, Outcome::Rows { len: 1 });
    assert_eq!((tuples[0].owner_country, tuples[0].target_country), (2, 1));
    let mut opinions = vec![OpinionRow::default(); 4];
    let both = Counting::new(&slice);
    walk(&both, BASE, GS, &mut opinions, &mut tuples, [true; 2]);
    assert!(counting.take()[2] < both.take()[2]);
}

mod dense {
    use super::super::*;

    const SLOT_FIELDS: [RecordField; 4] = [
        field(0, 0, 8),
        field(TARGET_COUNTRY, 8, 4),
        field(OPINION, 12, 4),
        field(SERIALIZABLE_RELATIONS, 16, 16),
    ];
    const SLOT_BYTES: usize = 32;
    const SLOT_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&SLOT_FIELDS, SLOT_BYTES).expect("valid record layout");

    pub fn walk(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        opinions: &mut Vec<OpinionRow>,
        tuples: &mut Vec<TupleRow>,
        resolved: &mut Resolver,
    ) -> Walk {
        let result = (|| {
            let countries = ccountry::Reader::new(memory, base, game_state)?;
            let status_vtable = (base + VTABLE_RVA.0) as u64;
            let mut highest_slot = 0;
            for slot in 1..countries.count() {
                if countries.at(slot)?.is_some() {
                    highest_slot = slot;
                }
            }
            let mut counters = Counters::default();
            let mut pointers = [0_usize; BATCH];
            let mut records = [0_u8; BATCH * SLOT_BYTES];
            resolved.begin(&countries);
            for slot in 1..countries.count() {
                let Some(country) = countries.at(slot)? else {
                    continue;
                };
                let diplomacy = diplomacy_root(memory, base, country)?;
                let matrix = Array::read(memory, diplomacy + cdiplomacy_status::RELATIONS, 8)
                    .ok_or("relation matrix unreadable or invalid")?;
                if matrix.count <= highest_slot {
                    return Err("relation matrix does not cover country ordinals");
                }
                let mut index = 1;
                while index < matrix.count {
                    let len = (matrix.count - index).min(BATCH as u32) as usize;
                    read_pointers(memory, matrix, index, &mut pointers[..len])?;
                    let copied = memory.copy_records(
                        RecordSources::Pointers(&pointers[..len]),
                        &SLOT_LAYOUT,
                        &mut records[..len * SLOT_BYTES],
                    );
                    for offset in 0..len {
                        if copied < (offset + 1) * SLOT_FIELDS.len() {
                            return Err("relation matrix slot fields unreadable");
                        }
                        let record = &records[offset * SLOT_BYTES..(offset + 1) * SLOT_BYTES];
                        if u64::from_le_bytes(record[..8].try_into().unwrap()) != status_vtable {
                            return Err("relation status vtable mismatch");
                        }
                        let target_country = u32::from_le_bytes(record[8..12].try_into().unwrap());
                        if resolved.resolve(&countries, target_country)?.canonical_slot
                            != index + offset as u32
                        {
                            return Err("relation status target differs from matrix slot");
                        }
                        let opinion = i32::from_le_bytes(record[12..16].try_into().unwrap());
                        if opinion != 0 {
                            counters.push_opinion(
                                &mut Buffer::Growing(&mut *opinions),
                                OpinionRow {
                                    owner_country: country.id,
                                    target_country,
                                    opinion,
                                },
                            )?;
                        }
                        let serializable = Array::from_header(record[16..].try_into().unwrap(), 8)
                            .ok_or("serializable relation vector invalid")?;
                        read_tuples(
                            memory,
                            base,
                            &countries,
                            country,
                            target_country,
                            serializable,
                            &mut Buffer::Growing(&mut *tuples),
                            &mut counters,
                            resolved,
                        )?;
                    }
                    index += len as u32;
                }
            }
            Ok(counters.walk())
        })();
        result.unwrap_or_else(|reason| Walk {
            opinions: Outcome::Unavailable { reason },
            tuples: Outcome::Unavailable { reason },
        })
    }
}


mod world {
    use super::super::*;
    use crate::{
        cgame_state, cguarantee_relation, cmarket_access_rights_relation,
        cmilitary_access_relation, cnon_aggression_pact_relation,
    };

    pub const BASE: usize = 0;
    pub const STATE: usize = 0x1000;
    const TABLE: usize = 0x10_0000;
    const LINKS: usize = 0x20_0000;
    const OBJECTS: usize = 0x100_0000;
    const OBJECT: usize = 0x1000;
    const DIPLOMACIES: usize = 0x10_0000_0000;
    const DIPLOMACY: usize = 0x400;
    const MATRICES: usize = 0x20_0000_0000;
    const OWNED: usize = 0x30_0000_0000;
    const OWNED_STRIDE: usize = 0x100;
    const RELATIONS: usize = 0x40_0000_0000;
    const RELATION: usize = 0x80;
    const SERIALIZED: usize = 0x50_0000_0000;
    const STATUSES: usize = 0x100_0000_0000;
    const NEIGHBOURS: usize = 3;
    const CLASSES: [(Rva, u32); 6] = [
        (cguarantee_relation::VTABLE_RVA, cguarantee_relation::TYPE),
        (
            cnon_aggression_pact_relation::VTABLE_RVA,
            cnon_aggression_pact_relation::TYPE,
        ),
        (cwar_relation::VTABLE_RVA, cwar_relation::TYPE),
        (
            cmarket_access_rights_relation::VTABLE_RVA,
            cmarket_access_rights_relation::TYPE,
        ),
        (csubject_relation::VTABLE_RVA, csubject_relation::TYPE_VALUE),
        (
            cmilitary_access_relation::VTABLE_RVA,
            cmilitary_access_relation::TYPE,
        ),
    ];

    pub struct World {
        countries: usize,
        day: std::cell::Cell<u32>,
    }

    impl World {
        pub fn new(countries: usize) -> Self {
            assert!(countries > NEIGHBOURS + 1);
            Self {
                countries,
                day: std::cell::Cell::new(0),
            }
        }

        fn written(owner: usize, target: usize) -> bool {
            (owner * 131 + target * 7).is_multiple_of(211)
        }

        pub fn next_day(&self) {
            self.day.set(self.day.get() + 1);
        }

        pub fn marks(&self) -> Vec<Mark> {
            let mut marks = Vec::new();
            for owner in 1..self.slots() {
                for target in 1..self.slots() {
                    if Self::written(owner, target) {
                        marks.push(Mark {
                            status: STATUSES + (owner * self.slots() + target) * OBJECT_SIZE,
                            diplomacy: DIPLOMACIES + owner * DIPLOMACY,
                        });
                    }
                }
            }
            marks
        }

        fn slots(&self) -> usize {
            self.countries + 1
        }

        fn aliased(&self, country: usize) -> bool {
            country.is_multiple_of(11)
        }

        fn alias(&self, country: usize) -> usize {
            self.slots() + country / 11 - 1
        }

        fn links(&self) -> usize {
            self.slots() + self.countries / 11
        }

        fn neighbour(&self, country: usize, step: usize) -> usize {
            (country - 1 + step) % self.countries + 1
        }

        fn extra(&self, first: usize) -> Option<usize> {
            first
                .is_multiple_of(5)
                .then(|| self.countries * NEIGHBOURS + first / 5 - 1)
        }

        fn relation(&self, id: usize) -> (usize, usize, usize) {
            if id < self.countries * NEIGHBOURS {
                let first = id / NEIGHBOURS + 1;
                (first, self.neighbour(first, id % NEIGHBOURS + 1), id % 6)
            } else {
                let first = (id - self.countries * NEIGHBOURS + 1) * 5;
                (first, self.neighbour(first, 1), 3)
            }
        }

        fn serialized(&self, owner: usize, target: usize, output: &mut [usize; 2]) -> usize {
            let step = (target + self.countries - owner) % self.countries;
            if owner == 0 || target == 0 || step == 0 || step > NEIGHBOURS {
                return 0;
            }
            output[0] = (owner - 1) * NEIGHBOURS + step - 1;
            match self.extra(owner).filter(|_| step == 1) {
                Some(extra) => {
                    output[1] = extra;
                    2
                }
                None => 1,
            }
        }

        fn owned(&self, country: usize, output: &mut [usize; 16]) -> usize {
            let mut len = 0;
            for step in 1..=NEIGHBOURS {
                let first = (country - 1 + self.countries - step) % self.countries + 1;
                output[len] = (first - 1) * NEIGHBOURS + step - 1;
                len += 1;
                if step == 1
                    && let Some(extra) = self.extra(first)
                {
                    output[len] = extra;
                    len += 1;
                }
            }
            for step in 1..=NEIGHBOURS {
                output[len] = (country - 1) * NEIGHBOURS + step - 1;
                len += 1;
            }
            if let Some(extra) = self.extra(country) {
                output[len] = extra;
                len += 1;
            }
            len
        }

        fn opinion(&self, owner: usize, target: usize) -> i32 {
            let base = if (owner * 7919 + target * 104_729).is_multiple_of(37) {
                ((owner + target) % 199) as i32 - 99
            } else {
                0
            };
            let written = Self::written(owner, target) && self.day.get() % 2 == 1;
            base + if written { 3 } else { 0 }
        }

        fn object(&self, address: usize, object: usize, output: &mut [u8]) {
            let offset = address - object;
            let end = offset + output.len();
            let mut field = |at: usize, value: &[u8]| {
                let (from, to) = (at.max(offset), (at + value.len()).min(end));
                if from < to {
                    output[from - offset..to - offset].copy_from_slice(&value[from - at..to - at]);
                }
            };
            if object >= STATUSES {
                let pair = (object - STATUSES) / OBJECT_SIZE;
                let (owner, target) = (pair / self.slots(), pair % self.slots());
                let hit = |at: usize, len: usize| at < end && offset < at + len;
                if hit(0, 8) {
                    field(0, &((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
                }
                if hit(TARGET_COUNTRY, 4) {
                    let raw = if self.aliased(target) && owner.is_multiple_of(4) {
                        self.alias(target)
                    } else {
                        target
                    };
                    field(TARGET_COUNTRY, &(raw as u32).to_le_bytes());
                }
                if hit(SERIALIZABLE_RELATIONS, 16) {
                    let mut relations = [0; 2];
                    let count = self.serialized(owner, target, &mut relations) as u32;
                    if count > 0 {
                        let data = (SERIALIZED + pair * 16) as u64;
                        field(SERIALIZABLE_RELATIONS, &data.to_le_bytes());
                        field(SERIALIZABLE_RELATIONS + 8, &count.to_le_bytes());
                        field(SERIALIZABLE_RELATIONS + 12, &count.to_le_bytes());
                    }
                }
                if hit(DIPLOMACY_STATUS, 8) {
                    let diplomacy = (DIPLOMACIES + owner * DIPLOMACY) as u64;
                    field(DIPLOMACY_STATUS, &diplomacy.to_le_bytes());
                }
                if hit(OPINION, 4) {
                    field(OPINION, &self.opinion(owner, target).to_le_bytes());
                }
            } else if object >= SERIALIZED {
                let pair = (object - SERIALIZED) / 16;
                let mut relations = [0; 2];
                let count =
                    self.serialized(pair / self.slots(), pair % self.slots(), &mut relations);
                for (index, relation) in relations[..count].iter().enumerate() {
                    field(
                        index * 8,
                        &((RELATIONS + relation * RELATION) as u64).to_le_bytes(),
                    );
                }
            } else if object >= RELATIONS {
                let id = (object - RELATIONS) / RELATION;
                let (first, second, class) = self.relation(id);
                let (vtable, type_id) = CLASSES[class];
                let first = if self.aliased(first) && id.is_multiple_of(2) {
                    self.alias(first)
                } else {
                    first
                };
                field(0, &((BASE + vtable.0) as u64).to_le_bytes());
                field(crelation::TYPE_ID, &type_id.to_le_bytes());
                field(crelation::FIRST_COUNTRY, &(first as u32).to_le_bytes());
                field(crelation::SECOND_COUNTRY, &(second as u32).to_le_bytes());
                field(crelation::START_HOUR, &(id as i32).to_le_bytes());
                field(crelation::END_HOUR, &(-(id as i32)).to_le_bytes());
                field(crelation::CANCEL, &[(id % 3) as u8]);
                field(crelation::RETIRED, &[u8::from(id.is_multiple_of(4)) * 2]);
            } else if object >= OWNED {
                let mut relations = [0; 16];
                let count = self.owned((object - OWNED) / OWNED_STRIDE, &mut relations);
                for (index, relation) in relations[..count].iter().enumerate() {
                    field(
                        index * 8,
                        &((RELATIONS + relation * RELATION) as u64).to_le_bytes(),
                    );
                }
            } else if object >= MATRICES {
                let owner = (object - MATRICES) / (self.slots() * 8);
                let first = (offset / 8).min(self.slots());
                let last = end.div_ceil(8).min(self.slots());
                for target in first..last {
                    let status = STATUSES + (owner * self.slots() + target) * OBJECT_SIZE;
                    field(target * 8, &(status as u64).to_le_bytes());
                }
            } else if object >= DIPLOMACIES {
                let owner = (object - DIPLOMACIES) / DIPLOMACY;
                let slots = self.slots() as u32;
                let mut owned = [0; 16];
                let owned = self.owned(owner, &mut owned) as u32;
                field(
                    0,
                    &((BASE + cdiplomacy_status::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                let matrix = MATRICES + owner * self.slots() * 8;
                field(cdiplomacy_status::RELATIONS, &(matrix as u64).to_le_bytes());
                field(cdiplomacy_status::RELATION_CAPACITY, &slots.to_le_bytes());
                field(cdiplomacy_status::RELATION_COUNT, &slots.to_le_bytes());
                let vector = OWNED + owner * OWNED_STRIDE;
                field(
                    cdiplomacy_status::OWNED_RELATIONS,
                    &(vector as u64).to_le_bytes(),
                );
                field(
                    cdiplomacy_status::OWNED_RELATION_CAPACITY,
                    &owned.to_le_bytes(),
                );
                field(
                    cdiplomacy_status::OWNED_RELATION_COUNT,
                    &owned.to_le_bytes(),
                );
                let country = OBJECTS + owner * OBJECT;
                field(cdiplomacy_status::COUNTRY, &(country as u64).to_le_bytes());
            } else if object >= OBJECTS {
                let slot = (object - OBJECTS) / OBJECT;
                field(0, &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes());
                field(ccountry::ID, &(slot as u32).to_le_bytes());
                let diplomacy = DIPLOMACIES + slot * DIPLOMACY;
                field(
                    ccountry::DIPLOMACY_STATUS,
                    &(diplomacy as u64).to_le_bytes(),
                );
            } else if object >= LINKS {
                let first = offset / 4;
                for raw in first..end.div_ceil(4).min(self.links()) {
                    let slot = if raw < self.slots() {
                        raw
                    } else {
                        (raw - self.slots() + 1) * 11
                    };
                    field(raw * 4, &(slot as u32).to_le_bytes());
                }
            } else if object >= TABLE {
                let first = (offset / 8).max(1);
                for slot in first..end.div_ceil(8).min(self.slots()) {
                    field(slot * 8, &((OBJECTS + slot * OBJECT) as u64).to_le_bytes());
                }
            } else {
                for (at, data, count) in [
                    (cgame_state::COUNTRIES, TABLE, self.slots()),
                    (cgame_state::COUNTRY_LINKS, LINKS, self.links()),
                ] {
                    field(at, &(data as u64).to_le_bytes());
                    field(at + 8, &(count as u32).to_le_bytes());
                    field(at + 12, &(count as u32).to_le_bytes());
                }
            }
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address >= STATUSES {
                let (pair, offset) = (
                    (address - STATUSES) / OBJECT_SIZE,
                    (address - STATUSES) % OBJECT_SIZE,
                );
                let (owner, target) = (pair / self.slots(), pair % self.slots());
                match (offset, output.len()) {
                    (0, 8) => {
                        output.copy_from_slice(&((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
                        return true;
                    }
                    (OPINION, 4) => {
                        output.copy_from_slice(&self.opinion(owner, target).to_le_bytes());
                        return true;
                    }
                    (TARGET_COUNTRY, 4) | (SERIALIZABLE_RELATIONS, 16) => {
                        let object = STATUSES + pair * OBJECT_SIZE;
                        output.fill(0);
                        self.object(address, object, output);
                        return true;
                    }
                    _ => {}
                }
            } else if (MATRICES..OWNED).contains(&address) && address.is_multiple_of(8) {
                let stride = self.slots() * 8;
                let owner = (address - MATRICES) / stride;
                let first = (address - MATRICES) % stride / 8;
                if output.len().is_multiple_of(8) && first + output.len() / 8 <= self.slots() {
                    for (index, chunk) in output.chunks_exact_mut(8).enumerate() {
                        let pair = owner * self.slots() + first + index;
                        let status = (STATUSES + pair * OBJECT_SIZE) as u64;
                        chunk.copy_from_slice(&status.to_le_bytes());
                    }
                    return true;
                }
            }
            output.fill(0);
            let object = if address >= STATUSES {
                STATUSES + (address - STATUSES) / OBJECT_SIZE * OBJECT_SIZE
            } else if address >= SERIALIZED {
                SERIALIZED + (address - SERIALIZED) / 16 * 16
            } else if address >= RELATIONS {
                RELATIONS + (address - RELATIONS) / RELATION * RELATION
            } else if address >= OWNED {
                OWNED + (address - OWNED) / OWNED_STRIDE * OWNED_STRIDE
            } else if address >= MATRICES {
                let stride = self.slots() * 8;
                MATRICES + (address - MATRICES) / stride * stride
            } else if address >= DIPLOMACIES {
                DIPLOMACIES + (address - DIPLOMACIES) / DIPLOMACY * DIPLOMACY
            } else if address >= OBJECTS {
                OBJECTS + (address - OBJECTS) / OBJECT * OBJECT
            } else if address >= LINKS {
                LINKS
            } else if address >= TABLE {
                TABLE
            } else if address >= STATE {
                STATE
            } else {
                return false;
            };
            self.object(address, object, output);
            true
        }
    }
}

struct Counting<'a, M> {
    inner: Lines<'a, M>,
    calls: std::cell::Cell<usize>,
}

struct Lines<'a, M> {
    memory: &'a M,
    bytes: std::cell::Cell<usize>,
    lines: std::cell::Cell<usize>,
}

impl<M: Memory> Memory for Lines<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !output.is_empty() {
            self.bytes.set(self.bytes.get() + output.len());
            let lines = (address + output.len() - 1) / 64 - address / 64 + 1;
            self.lines.set(self.lines.get() + lines);
        }
        self.memory.copy(address, output)
    }
}

impl<'a, M: Memory> Counting<'a, M> {
    fn new(memory: &'a M) -> Self {
        Self {
            inner: Lines {
                memory,
                bytes: Default::default(),
                lines: Default::default(),
            },
            calls: Default::default(),
        }
    }
    fn take(&self) -> [usize; 3] {
        [
            self.calls.replace(0),
            self.inner.bytes.replace(0),
            self.inner.lines.replace(0),
        ]
    }
}

impl<M: Memory> Memory for Counting<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_records(sources, layout, destination)
    }
}

#[test]
fn sliced_observed_days_equal_the_dense_walk() {
    let countries = 120;
    let world = world::World::new(countries);
    let memory = Counting::new(&world);
    let opinions_len = countries * countries / 30 + 64;
    let tuples_len = countries * 4;
    let rows = || {
        (
            vec![OpinionRow::default(); opinions_len],
            vec![TupleRow::default(); tuples_len],
        )
    };
    let links = countries + 1 + countries / 11;
    let mut resolver = Resolver::with_capacity(links);
    let token = (1, 1);
    let walk = |marks: &[Mark],
                baseline: &mut Baseline,
                select: &dyn Fn(u32) -> bool,
                resolver: &mut Resolver| {
        let mut output = rows();
        let result = walk_with(
            &memory,
            world::BASE,
            world::STATE,
            &mut output.0,
            &mut output.1,
            [true; 2],
            resolver,
            baseline,
            Some(Observation {
                token,
                marks: Some(marks),
            }),
            select,
        );
        (result, output, memory.take())
    };
    let mut baseline = Baseline::default();
    let mut load_cost = [0; 3];
    for _ in 0..3 {
        load_cost = walk(&[], &mut baseline, &|_| true, &mut resolver).2;
        baseline.grow().unwrap();
    }
    assert_eq!(baseline.valid_owners(), countries);
    world.next_day();
    let marks = world.marks();
    assert!(!marks.is_empty());
    baseline.reserve_marks(marks.len()).unwrap();
    let (mut opinions, mut tuples) = (Vec::new(), Vec::new());
    let mut hours = Vec::new();
    for hour in 0..24_u32 {
        let slice = move |owner: u32| owner % 24 == hour;
        let hour_marks: &[Mark] = if hour == 0 { &marks } else { &[] };
        let (observed, rows, cost) = walk(hour_marks, &mut baseline, &slice, &mut resolver);
        let (unobserved, reference, _) = walk(
            &[],
            &mut Baseline::default(),
            &slice,
            &mut Resolver::with_capacity(links),
        );
        assert_eq!(observed, unobserved, "{countries} hour {hour}");
        assert_eq!(
            rows.0[..observed.opinions.len()],
            reference.0[..unobserved.opinions.len()],
            "{countries} hour {hour}"
        );
        opinions.extend_from_slice(&rows.0[..observed.opinions.len()]);
        tuples.extend_from_slice(&rows.1[..observed.tuples.len()]);
        hours.push(cost);
    }
    assert!(baseline.counts().marks as usize >= marks.len() / 2);
    let mut expected = rows();
    let dense = dense::walk(
        &memory,
        world::BASE,
        world::STATE,
        &mut expected.0,
        &mut expected.1,
        &mut Resolver::with_capacity(links),
    );
    opinions.sort_by_key(|row| (row.owner_country, row.target_country));
    let mut dense_opinions = expected.0[..dense.opinions.len()].to_vec();
    dense_opinions.sort_by_key(|row| (row.owner_country, row.target_country));
    assert_eq!(opinions, dense_opinions, "{countries}");
    assert_eq!(tuples.len(), dense.tuples.len(), "{countries}");
    let worst = hours.iter().max_by_key(|cost| cost[2]).expect("24 hours");
    assert!(worst[2] * 8 < load_cost[2], "{worst:?} {load_cost:?}");
}

#[derive(Default)]
struct Sparse(std::collections::BTreeMap<usize, Vec<u8>>);

impl Sparse {
    fn object(&mut self, at: usize, len: usize) -> &mut Vec<u8> {
        self.0.entry(at).or_insert_with(|| vec![0; len])
    }
    fn p32(&mut self, object: usize, len: usize, at: usize, value: u32) {
        self.object(object, len)[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(&mut self, object: usize, len: usize, at: usize, value: u64) {
        self.object(object, len)[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(&mut self, object: usize, len: usize, at: usize, data: usize, values: &[u64]) {
        self.p64(object, len, at, data as u64);
        self.p32(object, len, at + 8, values.len() as u32);
        self.p32(object, len, at + 12, values.len() as u32);
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.0.insert(data, bytes);
    }
}

impl Memory for Sparse {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some((&start, bytes)) = self.0.range(..=address).next_back() else {
            return false;
        };
        let offset = address - start;
        let Some(source) = bytes.get(offset..offset + output.len()) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}

fn star(partners: usize) -> Sparse {
    const COUNTRY: usize = ccountry::CIVIL_WAR_LINK + 4;
    const DIPLOMACY: usize = cdiplomacy_status::COUNTRY + 8;
    const STATUS: usize = AIR_BASE_ACCESS + 8;
    const RELATION: usize = crelation::RETIRED + 1;
    let slots = partners + 2;
    let country = |slot: usize| 0x100_0000 + slot * 0x1000;
    let diplomacy = |slot: usize| 0x1000_0000 + slot * 0x1000;
    let matrix = |slot: usize| 0x2000_0000 + slot * 0x1_0000;
    let owned = |slot: usize| 0x4000_0000 + slot * 0x2_0000;
    let status = |owner: usize, target: usize| {
        0x8000_0000 + (owner.max(target) * 2 + usize::from(owner == 1)) * 0x1000
    };
    let relation = |partner: usize, class: usize| 0x1_0000_0000 + (partner * 2 + class) * 0x100;
    let mut world = Sparse::default();
    let mut table = vec![u64::MAX];
    table.extend((1..slots).map(|slot| country(slot) as u64));
    world.array(GS, 0x1000, cgame_state::COUNTRIES, 0x2000, &table);
    world.p64(GS, 0x1000, cgame_state::COUNTRY_LINKS, 0x40_0000);
    world.p32(GS, 0x1000, cgame_state::COUNTRY_LINKS + 8, slots as u32);
    world.p32(GS, 0x1000, cgame_state::COUNTRY_LINKS + 12, slots as u32);
    let links: Vec<u8> = (0..slots as u32).flat_map(u32::to_le_bytes).collect();
    world.0.insert(0x40_0000, links);
    let partner = |index: usize| index + 2;
    for slot in 1..slots {
        let (object, root) = (country(slot), diplomacy(slot));
        world.p64(object, COUNTRY, 0, (BASE + ccountry::VTABLE_RVA.0) as u64);
        world.p32(object, COUNTRY, ccountry::ID, slot as u32);
        world.p64(object, COUNTRY, ccountry::DIPLOMACY_STATUS, root as u64);
        world.p64(
            root,
            DIPLOMACY,
            0,
            (BASE + cdiplomacy_status::VTABLE_RVA.0) as u64,
        );
        world.p64(root, DIPLOMACY, cdiplomacy_status::COUNTRY, object as u64);
        let row: Vec<u64> = (0..slots)
            .map(|target| {
                if (slot == 1) != (target == 1) {
                    status(slot, target) as u64
                } else {
                    0
                }
            })
            .collect();
        let at = cdiplomacy_status::RELATIONS;
        world.array(root, DIPLOMACY, at, matrix(slot), &row);
        let relations: Vec<u64> = if slot == 1 {
            let order = |index: usize, step: usize| partner(index * step % partners);
            (0..partners)
                .map(|index| relation(order(index, 7919), 0) as u64)
                .chain((0..partners).map(|index| relation(order(index, 104_729), 1) as u64))
                .collect()
        } else {
            vec![relation(slot, 0) as u64, relation(slot, 1) as u64]
        };
        let at = cdiplomacy_status::OWNED_RELATIONS;
        world.array(root, DIPLOMACY, at, owned(slot), &relations);
    }
    for index in 0..partners {
        let other = partner(index);
        let classes = [
            cmilitary_access_relation::TYPE,
            cdocking_rights_relation::TYPE,
        ];
        for (class, type_id) in classes.into_iter().enumerate() {
            let object = relation(other, class);
            world.p32(object, RELATION, crelation::TYPE_ID, type_id);
            world.p32(object, RELATION, crelation::FIRST_COUNTRY, 1);
            world.p32(object, RELATION, crelation::SECOND_COUNTRY, other as u32);
        }
        for (owner, target) in [(1, other), (other, 1)] {
            let object = status(owner, target);
            world.p64(object, STATUS, 0, (BASE + VTABLE_RVA.0) as u64);
            world.p64(object, STATUS, DIPLOMACY_STATUS, diplomacy(owner) as u64);
            world.p32(object, STATUS, TARGET_COUNTRY, target as u32);
            world.p64(object, STATUS, MILITARY_ACCESS, relation(other, 0) as u64);
            world.p64(object, STATUS, DOCKING_RIGHTS, relation(other, 1) as u64);
        }
    }
    world
}

#[test]
fn reverse_military_presence_reads_the_whole_qword_without_following_the_pointer() {
    let mut bytes = reverse_fixture();
    let military_pointer = 1_u64 << 32;
    p64(
        &mut bytes,
        zero_status(0) + MILITARY_ACCESS,
        military_pointer,
    );
    let memory = ReverseTrace::new(&bytes, None);
    let source = reverse_source(&memory);
    let captured = source.cell(&memory, BASE, None, 0, None);
    let value = captured.value.unwrap();
    assert_eq!(value.military_access, Some(1));
    assert_eq!(value.target, Some(0));
    assert_eq!(
        value.subject,
        Some(ZeroSubjectInputs::default()),
        "flag0 facts remain available"
    );
    assert_eq!(captured.failures, [None; 4]);
    assert!(
        memory
            .calls
            .borrow()
            .contains(&(zero_status(0) + MILITARY_ACCESS, 8))
    );
    assert_eq!(
        memory.reads(military_pointer as usize),
        0,
        "presence never dereferences the relation"
    );
}

#[test]
fn reverse_military_fault_is_independent_and_active_war_skips_the_read() {
    let mut bytes = reverse_fixture();
    let memory = ReverseTrace::new(&bytes, Some(zero_status(0) + MILITARY_ACCESS));
    let source = reverse_source(&memory);
    let captured = source.cell(&memory, BASE, None, 0, None);
    let value = captured.value.unwrap();
    assert_eq!(value.military_access, None);
    assert_eq!(value.war, Some([0, 0]));
    assert_eq!(value.target, Some(0));
    assert_eq!(value.subject, Some(ZeroSubjectInputs::default()));
    assert_eq!(
        captured.failures[3],
        Some("reverse-zero military access pointer unreadable")
    );
    assert_eq!(memory.attempts.get(), 1);
    drop(source);
    drop(memory);
    p64(
        &mut bytes,
        zero_status(0) + WAR_RELATION,
        RELATION[0] as u64,
    );
    bytes[RELATION[0] + crelation::RETIRED] = 0;
    let memory = ReverseTrace::new(&bytes, Some(zero_status(0) + MILITARY_ACCESS));
    let source = reverse_source(&memory);
    let captured = source.cell(&memory, BASE, None, 0, None);
    assert_eq!(captured.value.unwrap().military_access, None);
    assert_eq!(captured.failures[3], None);
    assert_eq!(memory.attempts.get(), 0);
}

#[test]
fn refused_source_faction_does_not_suppress_military_presence() {
    let mut bytes = reverse_fixture();
    p64(&mut bytes, zero_status(0) + MILITARY_ACCESS, 1_u64 << 40);
    let memory = ReverseTrace::new(&bytes, Some(ZERO_DIPLOMACY + cdiplomacy_status::FACTION));
    let source = reverse_source(&memory);
    let captured = source.cell(&memory, BASE, None, 0, None);
    let value = captured.value.unwrap();
    assert_eq!(value.military_access, Some(1));
    assert_eq!(value.war, Some([0, 0]));
    assert_eq!((value.target, value.subject), (None, None));
    assert_eq!(captured.failures, [None; 4]);
    assert_eq!(memory.attempts.get(), 1);
}

fn forward_alias_zero_fixture() -> Vec<u8> {
    let mut bytes = reverse_fixture();
    links_vector(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        0x3100,
        &[0, 1, 2, 3, 2, 3, 0, 0],
    );
    p32(&mut bytes, status(0, 0) + TARGET_COUNTRY, 6);
    bytes
}

#[test]
fn forward_packet_alias_zero_reaches_war_and_whole_military_qword_before_old_guard() {
    for retired in [0, 0x80] {
        let mut bytes = forward_alias_zero_fixture();
        bytes[RELATION[0] + crelation::RETIRED] = retired;
        p64(&mut bytes, status(0, 0) + MILITARY_ACCESS, 1_u64 << 40);
        let memory = ReverseTrace::new(&bytes, None);
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let mut observed = ForwardZeroObservation::default();
        let mut targets = vec![99; 4];
        let old = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut targets,
            &mut observed,
        );
        assert_eq!(old.targets, Err("slot-0 relation target mismatch"));
        assert_eq!(old.zero_war, Err("slot-0 relation target mismatch"));
        assert_eq!(old.zero_subject, Err("slot-0 relation target mismatch"));
        assert_eq!(targets, [99; 4]);
        let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
        let value = packet.value.unwrap();
        assert_eq!(value.war, Some([1, retired]));
        assert_eq!(memory.reads(status(0, 0) + TARGET_COUNTRY), 1);
        assert_eq!(memory.reads(RELATION[0] + crelation::RETIRED), 1);
        assert_eq!(memory.reads(1_usize << 40), 0);
        if retired == 0 {
            assert_eq!(
                (value.target, value.military_access, value.subject),
                (None, None, None)
            );
            assert_eq!(memory.reads(status(0, 0) + MILITARY_ACCESS), 0);
            assert_eq!(memory.reads(status(0, 0) + SUBJECT_RELATION), 0);
        } else {
            assert_eq!(value.target, Some(6));
            assert_eq!(value.military_access, Some(1));
            assert_eq!(value.subject, Some(ZeroSubjectInputs::default()));
            assert!(
                memory
                    .calls
                    .borrow()
                    .contains(&(status(0, 0) + MILITARY_ACCESS, 8))
            );
        }
        assert_eq!(packet.failures, [None; 4]);
    }
}

#[test]
fn forward_packet_healthy_original_attempts_add_only_one_military_read() {
    let bytes = reverse_fixture();
    let memory = ReverseTrace::new(&bytes, None);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut observed = ForwardZeroObservation::default();
    let old = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![99; 4],
        &mut observed,
    );
    let before = memory.calls.borrow().len();
    let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
    let value = packet.value.unwrap();
    assert_eq!(value.war, old.zero_war.ok());
    assert_eq!(value.subject, old.zero_subject.ok());
    assert_eq!(value.target, Some(0));
    assert_eq!(value.military_access, Some(0));
    assert_eq!(
        &memory.calls.borrow()[before..],
        &[(status(0, 0) + MILITARY_ACCESS, 8)]
    );
}

#[test]
fn forward_packet_original_same_field_faults_are_never_retried_or_backfilled() {
    let mut bytes = reverse_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        2,
        0,
    );
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    for fault in [
        status(0, 0) + TARGET_COUNTRY,
        status(0, 0) + WAR_RELATION,
        RELATION[0] + crelation::RETIRED,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] + crelation::TYPE_ID,
        RELATION[1],
        RELATION[1] + crelation::FIRST_COUNTRY,
        RELATION[1] + crelation::SECOND_COUNTRY,
    ] {
        let memory = ReverseTrace::new(&bytes, Some(fault));
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let mut observed = ForwardZeroObservation::default();
        let old = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![99; 4],
            &mut observed,
        );
        assert!(
            old.zero_war.is_err()
                || old.zero_subject.is_err()
                || old.zero_subject.unwrap().read_refusal().is_some()
        );
        let old_attempts = observed;
        let _packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
        for (before, after) in [
            (old_attempts.war_pointer, observed.war_pointer),
            (old_attempts.subject_pointer, observed.subject_pointer),
        ] {
            if before.is_some() {
                assert_eq!(before, after);
            }
        }
        for (before, after) in [
            (old_attempts.subject_type, observed.subject_type),
            (old_attempts.first_party, observed.first_party),
            (old_attempts.second_party, observed.second_party),
        ] {
            if before.is_some() {
                assert_eq!(before, after);
            }
        }
        if old_attempts.war_retired.is_some() {
            assert_eq!(old_attempts.war_retired, observed.war_retired);
        }
        if old_attempts.subject_identity.is_some() {
            assert_eq!(old_attempts.subject_identity, observed.subject_identity);
        }
        assert_eq!(memory.reads(fault), 1, "same field {fault:x}");
        assert_eq!(memory.attempts.get(), 1);
    }
}

#[test]
fn forward_packet_new_pointed_faults_remain_sticky_for_equal_pointer_reverse_cells() {
    let mut bytes = forward_alias_zero_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        2,
        1,
    );
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    p64(
        &mut bytes,
        zero_status(1) + WAR_RELATION,
        RELATION[0] as u64,
    );
    for fault in [
        RELATION[0] + crelation::RETIRED,
        RELATION[1] + crelation::TYPE_ID,
        RELATION[1],
        RELATION[1] + crelation::FIRST_COUNTRY,
        RELATION[1] + crelation::SECOND_COUNTRY,
    ] {
        let memory = ReverseTrace::new(&bytes, Some(fault));
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let source = reverse_source(&memory);
        let mut observed = ForwardZeroObservation::default();
        let old = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![99; 4],
            &mut observed,
        );
        assert_eq!(old.zero_war, Err("slot-0 relation target mismatch"));
        let forward = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
        assert!(forward.failures.into_iter().any(|reason| reason.is_some()));
        let reverse = source.cell(&memory, BASE, Some(&countries), 1, Some(&observed));
        assert!(reverse.failures.into_iter().any(|reason| reason.is_some()));
        assert_eq!(memory.reads(fault), 1, "pointed field {fault:x}");
        assert_eq!(memory.attempts.get(), 1);
        assert_eq!(memory.reads(zero_status(1) + WAR_RELATION), 1);
    }
}

#[test]
fn forward_packet_unknown_war_skips_new_military_and_subject_without_repeating_target() {
    let bytes = forward_alias_zero_fixture();
    let memory = ReverseTrace::new(&bytes, Some(status(0, 0) + WAR_RELATION));
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut observed = ForwardZeroObservation::default();
    let _old = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
    assert_eq!(
        packet.value.unwrap(),
        ReverseZeroVisibilityInputs::default()
    );
    assert_eq!(
        packet.failures[1],
        Some("forward-zero war pointer unreadable")
    );
    assert_eq!(memory.reads(status(0, 0) + TARGET_COUNTRY), 1);
    assert_eq!(memory.reads(status(0, 0) + MILITARY_ACCESS), 0);
    assert_eq!(memory.reads(status(0, 0) + SUBJECT_RELATION), 0);
}

#[test]
fn forward_packet_subject_or_uses_raw_zero_and_positive_aliases_in_native_order() {
    for (first, second, second_reads) in [(6, 2, 0), (7, 2, 0), (0, 6, 1)] {
        let mut bytes = forward_alias_zero_fixture();
        relation(
            &mut bytes,
            RELATION[1],
            csubject_relation::VTABLE_RVA,
            csubject_relation::TYPE_VALUE,
            first,
            second,
        );
        p64(
            &mut bytes,
            status(0, 0) + SUBJECT_RELATION,
            RELATION[1] as u64,
        );
        let memory = ReverseTrace::new(&bytes, None);
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let mut observed = ForwardZeroObservation::default();
        let old = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![0; 4],
            &mut observed,
        );
        assert_eq!(old.zero_subject, Err("slot-0 relation target mismatch"));
        let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
        assert_eq!(
            packet.value.unwrap().subject.unwrap().parties,
            Some([Some(first), (second_reads != 0).then_some(second)])
        );
        assert_eq!(
            memory.reads(RELATION[1] + crelation::SECOND_COUNTRY),
            second_reads
        );
        assert_eq!(packet.failures, [None; 4]);
    }
}

#[test]
fn forward_packet_partial_first_failure_retains_known_second_raw_match() {
    let mut bytes = forward_alias_zero_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        2,
        6,
    );
    p64(
        &mut bytes,
        status(0, 0) + SUBJECT_RELATION,
        RELATION[1] as u64,
    );
    let memory = ReverseTrace::new(&bytes, Some(RELATION[1] + crelation::FIRST_COUNTRY));
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut observed = ForwardZeroObservation::default();
    let _old = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
    assert_eq!(
        packet.value.unwrap().subject.unwrap().parties,
        Some([None, Some(6)])
    );
    assert_eq!(
        packet.failures[2],
        Some("reverse-zero subject first party unreadable")
    );
    assert_eq!(memory.reads(RELATION[1] + crelation::FIRST_COUNTRY), 1);
    assert_eq!(memory.reads(RELATION[1] + crelation::SECOND_COUNTRY), 1);
}

#[test]
fn forward_packet_military_failure_retains_independent_alias_target_and_subject() {
    let bytes = forward_alias_zero_fixture();
    let memory = ReverseTrace::new(&bytes, Some(status(0, 0) + MILITARY_ACCESS));
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut observed = ForwardZeroObservation::default();
    let old = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
    let value = packet.value.unwrap();
    assert_eq!(old.zero_war, Err("slot-0 relation target mismatch"));
    assert_eq!(value.target, Some(6));
    assert_eq!(value.war, Some([1, 0x7f]));
    assert_eq!(value.subject, Some(ZeroSubjectInputs::default()));
    assert_eq!(value.military_access, None);
    assert_eq!(
        packet.failures[3],
        Some("forward-zero military access pointer unreadable")
    );
    assert_eq!(memory.attempts.get(), 1);
}

#[test]
fn forward_packet_unknown_positive_target_link_is_not_assumed_zero() {
    let mut bytes = forward_alias_zero_fixture();
    p32(&mut bytes, status(0, 0) + TARGET_COUNTRY, 99);
    p64(&mut bytes, status(0, 0) + MILITARY_ACCESS, 1_u64 << 40);
    let memory = ReverseTrace::new(&bytes, None);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut observed = ForwardZeroObservation::default();
    let _old = co_belligerent_inputs_observed(
        &memory,
        BASE,
        &countries,
        country(0),
        &mut vec![0; 4],
        &mut observed,
    );
    let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
    let value = packet.value.unwrap();
    assert_eq!(value.target, None);
    assert!(packet.failures[0].is_some());
    assert_eq!(value.military_access, Some(1));
    assert_eq!(value.subject, Some(ZeroSubjectInputs::default()));
    assert_eq!(memory.reads(status(0, 0) + TARGET_COUNTRY), 1);
}

#[test]
fn forward_packet_failed_status_controls_refuse_without_reacquiring_root_or_header() {
    let bytes = reverse_fixture();
    for fault in [
        COUNTRIES[0] + ccountry::DIPLOMACY_STATUS,
        DIPLOMACY[0] + cdiplomacy_status::RELATIONS,
        status(0, 0),
        status(0, 0) + DIPLOMACY_STATUS,
    ] {
        let memory = ReverseTrace::new(&bytes, Some(fault));
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let mut observed = ForwardZeroObservation::default();
        let old = co_belligerent_inputs_observed(
            &memory,
            BASE,
            &countries,
            country(0),
            &mut vec![0; 4],
            &mut observed,
        );
        let before = memory.calls.borrow().len();
        let packet = forward_zero_visibility_inputs(&memory, BASE, &countries, &mut observed);
        assert_eq!(packet.value.err(), old.zero_war.err());
        assert_eq!(packet.failures, [None; 4]);
        assert_eq!(memory.calls.borrow().len(), before);
        assert_eq!(memory.reads(fault), 1);
        assert_eq!(memory.attempts.get(), 1);
    }
}

mod selected_enemies;

#[test]
fn growing_relation_rows_complete_the_original_walk() {
    let world = world::World::new(120);
    let memory = Counting::new(&world);
    let mut expected_opinions = vec![OpinionRow::default(); 120];
    let mut expected_tuples = vec![TupleRow::default(); 120];
    let expected = walk_with(
        &memory,
        world::BASE,
        world::STATE,
        &mut expected_opinions,
        &mut expected_tuples,
        [true; 2],
        &mut Resolver::default(),
        &mut Baseline::default(),
        None,
        |owner| owner == 1,
    );
    let expected_reads = memory.take();
    assert!(matches!(expected.opinions, Outcome::Rows { len } if len > 1));
    assert!(matches!(expected.tuples, Outcome::Rows { len } if len > 1));
    let mut opinions = vec![OpinionRow::default(); 1];
    let mut tuples = vec![TupleRow::default(); 1];
    let actual = walk_with(
        &memory,
        world::BASE,
        world::STATE,
        &mut opinions,
        &mut tuples,
        [true; 2],
        &mut Resolver::default(),
        &mut Baseline::default(),
        None,
        |owner| owner == 1,
    );
    assert_eq!(actual, expected);
    assert_eq!(
        &opinions[..actual.opinions.len()],
        &expected_opinions[..expected.opinions.len()]
    );
    assert_eq!(
        &tuples[..actual.tuples.len()],
        &expected_tuples[..expected.tuples.len()]
    );
    assert_eq!(
        memory.take(),
        expected_reads,
        "growth must not restart native reads"
    );
}

#[test]
fn signed_country_slots_above_u16_reach_links_tuples_and_co_belligerents() {
    const HIGH_SLOT: usize = 70_000;
    let mut bytes = fixture();
    let table = bytes.len();
    let links = table + (HIGH_SLOT + 1) * 8;
    let matrices = links + (HIGH_SLOT + 1) * 4;
    bytes.resize(matrices + 3 * (HIGH_SLOT + 1) * 8, 0);
    for (offset, data) in [
        (cgame_state::COUNTRIES, table),
        (cgame_state::COUNTRY_LINKS, links),
    ] {
        p64(&mut bytes, GS + offset, data as u64);
        p32(&mut bytes, GS + offset + 8, (HIGH_SLOT + 1) as u32);
        p32(&mut bytes, GS + offset + 12, (HIGH_SLOT + 1) as u32);
    }
    for (index, slot) in [1, 2, HIGH_SLOT].into_iter().enumerate() {
        p64(&mut bytes, table + slot * 8, COUNTRIES[index] as u64);
        p32(&mut bytes, links + slot * 4, slot as u32);
        p32(&mut bytes, COUNTRIES[index] + ccountry::ID, slot as u32);
        let data = matrices + index * (HIGH_SLOT + 1) * 8;
        for slot in 0..=HIGH_SLOT {
            p64(&mut bytes, data + slot * 8, status(index, 0) as u64);
        }
        p32(&mut bytes, status(index, 0) + TARGET_COUNTRY, 0);
        p64(
            &mut bytes,
            DIPLOMACY[index] + cdiplomacy_status::RELATIONS,
            data as u64,
        );
        p32(
            &mut bytes,
            DIPLOMACY[index] + cdiplomacy_status::RELATIONS + 8,
            (HIGH_SLOT + 1) as u32,
        );
        p32(
            &mut bytes,
            DIPLOMACY[index] + cdiplomacy_status::RELATIONS + 12,
            (HIGH_SLOT + 1) as u32,
        );
        for (target, old) in [(1, 1), (2, 2), (HIGH_SLOT, 3)] {
            p64(&mut bytes, data + target * 8, status(index, old) as u64);
            p32(
                &mut bytes,
                status(index, old) + TARGET_COUNTRY,
                target as u32,
            );
        }
    }
    relation(
        &mut bytes,
        RELATION[0],
        cattache_relation::VTABLE_RVA,
        cattache_relation::TYPE,
        1,
        HIGH_SLOT as u32,
    );
    for owner in [0, 2] {
        own(&mut bytes, owner, &[RELATION[0]]);
    }
    p64(&mut bytes, status(2, 1) + ATTACHE_LINK, RELATION[0] as u64);
    array(
        &mut bytes,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64],
    );
    let mut link_output = vec![LinkRow::default(); 4];
    assert_eq!(
        link_rows(&bytes, &mut link_output, &mut Resolver::default()),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(link_output[0].country, HIGH_SLOT as u32);
    let mut opinions = Vec::new();
    let mut tuples = Vec::new();
    let report = walk_with(
        &memory(&bytes),
        BASE,
        GS,
        &mut opinions,
        &mut tuples,
        [false, true],
        &mut Resolver::default(),
        &mut Baseline::default(),
        None,
        |_| true,
    );
    assert_eq!(report.tuples, Outcome::Rows { len: 1 });
    assert_eq!(tuples[0].target_country, HIGH_SLOT as u32);
    array(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[HIGH_SLOT as u64],
    );
    p32(&mut bytes, 0x3600, HIGH_SLOT as u32);
    bytes[status(0, 3) + CO_BELLIGERENT] = 1;
    let source = memory(&bytes);
    let mut memo = ccountry::NativeReferenceMemo::try_with_capacity(0, 0).unwrap();
    memo.reset(BASE, GS);
    let countries = memo.view(&source, BASE, GS);
    let mut co = vec![0; 4];
    assert_eq!(
        co_belligerents_native(&source, BASE, &countries, country(0), &mut co),
        Ok((1, 1))
    );
    assert_eq!(co[0], HIGH_SLOT as u32);
}

struct CallLog<'a> {
    memory: SliceMemory<'a>,
    calls: std::cell::RefCell<Vec<Vec<usize>>>,
}

impl Memory for CallLog<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push(vec![address]);
        self.memory.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.borrow_mut().push(
            addresses
                .iter()
                .zip(destinations.iter())
                .filter(|(_, destination)| !destination.is_empty())
                .map(|(address, _)| *address)
                .collect(),
        );
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !destination.is_empty() && (address == 0 || !self.memory.copy(address, destination))
            {
                return index;
            }
        }
        N
    }
}

#[test]
fn a_co_candidates_status_fields_are_copied_in_one_guarded_call() {
    let bytes = co_inputs_fixture();
    let memory = CallLog {
        memory: memory(&bytes),
        calls: Default::default(),
    };
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut slots = vec![99_u32; 4];
    let inputs = co_belligerent_inputs(&memory, BASE, &countries, country(0), &mut slots);
    assert_eq!(inputs.targets, Ok(3));
    assert_eq!(slots, [0, 2, 3, 99]);
    for target in [2, 3] {
        let object = status(0, target)..status(0, target) + CO_BELLIGERENT + 1;
        let calls = memory
            .calls
            .borrow()
            .iter()
            .filter(|call| call.iter().any(|at| object.contains(at)))
            .count();
        assert_eq!(calls, 1, "status of slot {target}");
    }
}

struct Hints<'a> {
    memory: SliceMemory<'a>,
    log: std::cell::RefCell<Vec<(bool, usize)>>,
}
impl Memory for Hints<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.log.borrow_mut().push((false, address));
        self.memory.copy(address, output)
    }
    fn prefetch(&self, address: usize, _len: usize) {
        self.log.borrow_mut().push((true, address));
    }
}

#[test]
fn owners_enemy_headers_and_owned_pointers_are_hinted_before_their_reads() {
    let bytes = transport_subject_fixture();
    let hints = Hints {
        memory: memory(&bytes),
        log: Default::default(),
    };
    let mut rows = vec![LinkRow::default(); 8];
    let walk = transport_links(&hints, &mut rows);
    let mut plain = vec![LinkRow::default(); 8];
    assert_eq!(walk, transport_links(&memory(&bytes), &mut plain));
    assert_eq!(rows, plain);
    let log = hints.log.into_inner();
    let first = |hint: bool, address: usize| log.iter().position(|&entry| entry == (hint, address));
    for diplomacy in DIPLOMACY {
        let enemies = diplomacy + cdiplomacy_status::ENEMY_COUNTRIES;
        assert!(first(true, enemies).unwrap() < first(false, enemies).unwrap());
    }
    for owner in 0..2 {
        let owned = 0x3400 + owner * 0x40;
        assert!(first(true, owned).unwrap() < first(false, owned).unwrap());
    }
}

struct HintedTrace<'a> {
    trace: ReverseTrace<'a>,
    hints: std::cell::Cell<usize>,
}
impl Memory for HintedTrace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.trace.copy(address, output)
    }
    fn prefetch(&self, _address: usize, _len: usize) {
        self.hints.set(self.hints.get() + 1);
    }
}

#[test]
fn relation_hints_leave_every_read_and_a_one_shot_fault_unchanged() {
    let bytes = transport_subject_fixture();
    for fault in [
        None,
        Some(0x3440),
        Some(DIPLOMACY[1] + cdiplomacy_status::ENEMY_COUNTRIES),
    ] {
        let hinted = HintedTrace {
            trace: ReverseTrace::new(&bytes, fault),
            hints: std::cell::Cell::new(0),
        };
        let plain = ReverseTrace::new(&bytes, fault);
        let (mut rows, mut plain_rows) = (Vec::new(), Vec::new());
        assert_eq!(
            transport_links(&hinted, &mut rows),
            transport_links(&plain, &mut plain_rows),
            "{fault:x?}"
        );
        assert_eq!(rows, plain_rows);
        assert_eq!(*hinted.trace.calls.borrow(), *plain.calls.borrow());
        assert!(hinted.hints.get() >= 5);
        if let Some(at) = fault {
            assert!(hinted.trace.reads(at) >= 1);
        }
    }
}

struct SerialCandidates<'a, M>(&'a M);
impl<M: Memory> Memory for SerialCandidates<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        if destination.len() == sources.len() * 8 {
            return 0;
        }
        self.0.copy_records(sources, layout, destination)
    }
}

struct StarFaults<'a> {
    world: &'a Sparse,
    persistent: Vec<usize>,
    once: std::cell::RefCell<Vec<usize>>,
}
impl Memory for StarFaults<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let covers = |at: usize| address <= at && at - address < output.len();
        if self.persistent.iter().any(|&at| covers(at)) {
            return false;
        }
        let mut once = self.once.borrow_mut();
        if let Some(index) = once.iter().position(|&at| covers(at)) {
            once.remove(index);
            return false;
        }
        drop(once);
        self.world.copy(address, output)
    }
}

fn star_links(memory: &impl Memory, partners: usize) -> (Outcome, Vec<LinkRow>) {
    let mut resolver = Resolver::with_capacity(partners + 2);
    let mut rows = vec![LinkRow::default(); partners * 2];
    let outcome = links_observed_inner(memory, BASE, GS, crate::walk::Buffer::Growing(&mut rows), WarOutput {
            rows: &mut Vec::new(),
            countries: &mut Vec::new(),
        }.into(), &mut resolver, None, None)
    .links;
    (outcome, rows)
}

fn star_transport(
    memory: &impl Memory,
    partners: usize,
) -> (LinkWalk, Vec<LinkRow>, Vec<WarRow>, Vec<u32>) {
    let mut resolver = Resolver::with_capacity(partners + 2);
    let mut rows = vec![LinkRow::default(); partners * 4];
    let mut wars = vec![WarRow::default(); partners + 2];
    let mut countries = vec![0_u32; 16];
    let walk = links_observed_transport(
        memory,
        BASE,
        GS,
        &mut rows,
        WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        }
        .into(),
        &mut resolver,
        73,
    );
    (walk, rows, wars, countries)
}

fn star_cell(target: usize) -> usize {
    0x2000_0000 + 0x1_0000 + target * 8
}
fn star_status(target: usize) -> usize {
    0x8000_0000 + (target * 2 + 1) * 0x1000
}

#[test]
fn gathered_relation_candidates_equal_the_serial_walk() {
    for partners in [3, 70] {
        let world = star(partners);
        let serial = serially(|| star_links(&world, partners));
        let gathered = star_links(&world, partners);
        assert_eq!(gathered.0, Outcome::Rows { len: partners * 2 });
        assert_eq!(gathered, serial, "{partners}");
        let gathered = star_transport(&world, partners);
        assert!(matches!(gathered.0.links, Outcome::Rows { .. }));
        assert_eq!(
            gathered,
            serially(|| star_transport(&world, partners)),
            "{partners}"
        );
        let before = crate::batch_fallback::count(crate::batch_fallback::Batch::RelationLinks);
        assert_eq!(star_links(&SerialCandidates(&world), partners), serial);
        assert!(crate::batch_fallback::count(crate::batch_fallback::Batch::RelationLinks) > before);
    }
}

#[test]
fn a_persistent_candidate_fault_gives_the_serial_result() {
    let partners = 200;
    let world = star(partners);
    for at in [star_cell(57), star_status(57)] {
        let faults = || StarFaults {
            world: &world,
            persistent: vec![at],
            once: Default::default(),
        };
        let before = crate::batch_fallback::count(crate::batch_fallback::Batch::RelationLinks);
        let gathered = star_links(&faults(), partners);
        assert!(
            crate::batch_fallback::count(crate::batch_fallback::Batch::RelationLinks) > before,
            "{at:#x}"
        );
        assert!(matches!(gathered.0, Outcome::Unavailable { .. }), "{at:#x}");
        assert_eq!(
            gathered,
            serially(|| star_links(&faults(), partners)),
            "{at:#x}"
        );
    }
}

#[test]
fn a_one_shot_candidate_fault_refuses_no_more_than_the_serial_walk() {
    let partners = 200;
    let world = star(partners);
    let healthy = star_links(&world, partners);
    for at in [star_cell(57), star_status(57)] {
        let faults = StarFaults {
            world: &world,
            persistent: Vec::new(),
            once: std::cell::RefCell::new(vec![at]),
        };
        assert_eq!(star_links(&faults, partners), healthy, "{at:#x}");
    }
}
