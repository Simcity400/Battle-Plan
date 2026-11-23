use super::*;
use std::cell::RefCell;

struct Trace<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl Memory for Trace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            return false;
        }
        memory(self.bytes).copy(address, output)
    }
}
fn current(source: &impl Memory, rows: &mut Vec<WarRow>, words: &mut Vec<u32>) -> LinkWalk {
    links_observed_inner(source, BASE, GS, engine_native_memory::walk::Buffer::Growing(&mut vec![LinkRow::default(); 4]), WarOutput {
            rows,
            countries: words,
        }.into(), &mut Resolver::default(), Some(77), None)
}
fn trace(bytes: &[u8], fault: Option<usize>) -> Trace<'_> {
    Trace {
        bytes,
        fault,
        reads: Default::default(),
    }
}
fn reserved(bytes: &mut [u8]) {
    const ROOT: usize = 0x7000;
    const DIP: usize = 0xc000;
    p64(bytes, 0x3000, ROOT as u64);
    p64(bytes, ROOT, ccountry::VTABLE_RVA.0 as u64);
    p32(bytes, ROOT + ccountry::ID, 0);
    p64(bytes, ROOT + ccountry::DIPLOMACY_STATUS, DIP as u64);
    bytes[ROOT + ccountry::CAN_OCCUPY_NON_WAR] = 0x80;
    p64(bytes, DIP, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
    p64(bytes, DIP + engine_native_parts::cdiplomacy_status_layout::COUNTRY, ROOT as u64);
    links_vector(
        bytes,
        DIP + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
        0x3800,
        &[3, 0, 3],
    );
}
fn required_enemy_fixture(retired: bool) -> Vec<u8> {
    const WAR: usize = 0x1d000;
    let mut bytes = war_fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
        0x36c0,
        &[3],
    );
    if retired {
        relation(
            &mut bytes,
            WAR,
            cwar_relation::VTABLE_RVA,
            cwar_relation::TYPE,
            2,
            3,
        );
        bytes[WAR + crelation::RETIRED] = 1;
        own(&mut bytes, 1, &[RELATION[0], WAR]);
    } else {
        p32(&mut bytes, COUNTRIES[1] + ccountry::CIVIL_WAR_LINK, 1);
    }
    relation(
        &mut bytes,
        RELATION[1],
        cmilitary_access_relation::VTABLE_RVA,
        cmilitary_access_relation::TYPE,
        3,
        1,
    );
    own(&mut bytes, 0, &RELATION);
    own(
        &mut bytes,
        2,
        if retired {
            &[WAR, RELATION[1]]
        } else {
            &[RELATION[1]]
        },
    );
    p64(
        &mut bytes,
        status(2, 1) + MILITARY_ACCESS,
        RELATION[1] as u64,
    );
    bytes
}
#[test]
fn current_keeps_ordinary_zero_rule_receivers_repeats_zero_and_one_original_read() {
    let mut bytes = war_fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
        0x36c0,
        &[3, 0, 3, 1],
    );
    p32(&mut bytes, COUNTRIES[1] + ccountry::CIVIL_WAR_LINK, 1);
    let source = trace(&bytes, None);
    let mut rows = vec![WarRow::default(); 4];
    let mut words = vec![99; 8];
    let walk = current(&source, &mut rows, &mut words);
    assert_eq!(walk.links, Outcome::Rows { len: 1 });
    assert_eq!(walk.wars, Outcome::Rows { len: 3 });
    assert_eq!((walk.countries_len, walk.required_countries), (7, 7));
    assert_eq!(rows[1].country, 2);
    assert_eq!(rows[1].ordered, Some((1, 4)));
    assert_eq!((rows[1].enemies_start, rows[1].enemies_len), (1, 4));
    assert_eq!(words[..7], [3, 3, 0, 3, 1, 1, 2]);
    assert_eq!(rows[1].ordered_hour, Some(77));
    for at in [DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES, 0x36c0] {
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|(start, len)| *start <= at && at - *start < *len)
                .count(),
            1
        );
    }
    assert_eq!(walk.ordered_refusals, 1);
}
#[test]
fn current_reserved_receiver_is_actual_zero_and_reuses_its_authentication() {
    let mut bytes = war_fixture();
    reserved(&mut bytes);
    let source = trace(&bytes, None);
    let mut rows = vec![WarRow::default(); 4];
    let mut words = vec![99; 8];
    let walk = current(&source, &mut rows, &mut words);
    assert_eq!(walk.wars, Outcome::Rows { len: 4 });
    assert_eq!(walk.ordered_refusals, 0);
    assert_eq!((rows[0].country, rows[0].can_occupy_non_war), (0, 0x80));
    assert_eq!(rows[0].ordered, Some((0, 3)));
    assert!(!rows[0].enemies_read);
    assert_eq!(words[..3], [3, 0, 3]);
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x3000)
            .count(),
        1
    );
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0xc000 + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES)
            .count(),
        1
    );
}
#[test]
fn current_payload_faults_refuse_locally_without_retry_or_invented_empty() {
    let mut bytes = war_fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
        0x36c0,
        &[3, 0, 3],
    );
    for (capacity, fault) in [(8, Some(0x36c0))] {
        let source = trace(&bytes, fault);
        let mut rows = vec![WarRow::default(); 4];
        let mut words = vec![99; capacity];
        let walk = current(&source, &mut rows, &mut words);
        assert_eq!(walk.links, Outcome::Rows { len: 1 });
        assert_eq!(walk.wars, Outcome::Rows { len: 3 });
        assert_eq!(walk.required_countries, 6);
        assert_eq!(rows[1].ordered, None);
        assert!(rows[1].ordered_failure.is_some());
        assert_eq!(rows[1].ordered_hour, Some(77));
        if fault.is_some() {
            assert_eq!(
                source
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(at, _)| *at == 0x36c0)
                    .count(),
                1
            );
            assert_eq!(rows[2].ordered, Some((1, 2)));
        } else {
            assert_eq!(walk.countries_len, 0);
        }
    }
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
        0x36c0,
        &[],
    );
    let source = trace(&bytes, None);
    let mut rows = vec![WarRow::default(); 4];
    current(&source, &mut rows, &mut Vec::new());
    assert_eq!(rows[1].ordered.map(|(_, len)| len), Some(0));
}
#[test]
fn current_required_enemy_faults_keep_named_failure_and_later_links() {
    let header = DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES;
    for retired in [false, true] {
        let bytes = required_enemy_fixture(retired);
        for (fault, reason, required) in [
            (header, "ordered enemy vector unreadable or invalid", 3),
            (0x36c0, "diplomacy country vector unreadable", 4),
        ] {
            let source = trace(&bytes, Some(fault));
            let mut links = vec![LinkRow::default(); 4];
            let mut rows = vec![WarRow::default(); 4];
            let mut words = vec![99; 8];
            let walk = links_observed_inner(&source, BASE, GS, engine_native_memory::walk::Buffer::Growing(&mut links), WarOutput {
                    rows: &mut rows,
                    countries: &mut words,
                }.into(), &mut Resolver::default(), Some(77), None);
            assert_eq!(walk.wars, Outcome::Unavailable { reason });
            assert_eq!(walk.links, Outcome::Rows { len: 2 });
            assert_eq!(
                (links[1].country, links[1].target, links[1].links),
                (3, 1, LINK_MILITARY_ACCESS)
            );
            assert_eq!(walk.ordered_refusals, 2);
            assert_eq!((walk.countries_len, walk.required_countries), (3, required));
            assert_eq!(rows[1].country, 3);
            assert_eq!(rows[1].ordered, Some((1, 2)));
            assert_eq!(words[..3], [3, 1, 2]);
            for at in [header, fault, 0x3680] {
                assert_eq!(
                    source
                        .reads
                        .borrow()
                        .iter()
                        .filter(|(start, len)| *start <= at && at - *start < *len)
                        .count(),
                    1,
                );
            }
            if fault == header {
                assert!(!source.reads.borrow().iter().any(|(at, _)| *at == 0x36c0));
            }
        }
    }
}

#[test]
fn legacy_required_enemy_faults_keep_named_failure_and_later_links() {
    let header = DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES;
    for retired in [false, true] {
        let bytes = required_enemy_fixture(retired);
        for (fault, reason) in [
            (header, "enemy vector unreadable or invalid"),
            (0x36c0, "diplomacy country vector unreadable"),
        ] {
            let source = trace(&bytes, Some(fault));
            let mut link_rows = vec![LinkRow::default(); 4];
            let walk = links_observed_inner(&source, BASE, GS, engine_native_memory::walk::Buffer::Growing(&mut link_rows), WarOutput {
                    rows: &mut vec![WarRow::default(); 4],
                    countries: &mut vec![0; 8],
                }.into(), &mut Resolver::default(), None, None);
            assert_eq!(walk.wars, Outcome::Unavailable { reason });
            assert_eq!(walk.links, Outcome::Rows { len: 2 });
            assert_eq!(
                (
                    link_rows[1].country,
                    link_rows[1].target,
                    link_rows[1].links
                ),
                (3, 1, LINK_MILITARY_ACCESS)
            );
            assert_eq!(walk.ordered_refusals, 0);
            assert_eq!(
                source
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(at, _)| *at == fault)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn current_required_enemy_fault_keeps_first_reason_before_later_fault() {
    let mut bytes = required_enemy_fixture(false);
    p32(&mut bytes, COUNTRIES[2] + ccountry::CIVIL_WAR_LINK, 1);
    let later_header = DIPLOMACY[2] + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES;
    p32(&mut bytes, later_header + engine_native_memory::cpdx_array::CAPACITY, 0);
    let source = trace(&bytes, Some(0x36c0));
    let walk = current(&source, &mut Vec::new(), &mut Vec::new());
    assert_eq!(
        walk.wars,
        Outcome::Unavailable {
            reason: "diplomacy country vector unreadable"
        }
    );
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x36c0)
            .count(),
        1
    );
    assert_eq!(walk.links, Outcome::Rows { len: 2 });
    assert_eq!(walk.ordered_refusals, 3);
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == later_header)
            .count(),
        1
    );
}

#[test]
fn current_required_enemy_failure_survives_independent_later_link_failure() {
    let mut bytes = required_enemy_fixture(false);
    p64(&mut bytes, status(2, 1), 0);
    let source = trace(&bytes, Some(0x36c0));
    let walk = current(&source, &mut vec![WarRow::default(); 4], &mut vec![0; 8]);
    assert_eq!(
        walk.links,
        Outcome::Unavailable {
            reason: "relation status vtable mismatch"
        }
    );
    assert_eq!(
        walk.wars,
        Outcome::Unavailable {
            reason: "diplomacy country vector unreadable"
        }
    );
    assert_eq!(walk.ordered_refusals, 2);
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x36c0)
            .count(),
        1
    );
}

